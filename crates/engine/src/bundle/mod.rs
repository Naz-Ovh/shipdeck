//! Packing a release into one `.tar.gz` in memory: the binary, the files
//! `pipeline.pack.files` picks, `release.json` and the systemd unit.

pub mod files;
pub mod unit;

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::config::{Deployable, release_path};
use crate::release::{MANIFEST_FILE, Manifest};
use files::{Kind, Selection};

/// A packed release, ready to upload.
#[derive(Debug)]
pub struct Bundle {
    pub data: Vec<u8>,
    pub files: usize,
    /// Size before compression.
    pub input_bytes: u64,
    /// Project files a file from the deploy folder replaced.
    pub replaced: Vec<String>,
}

enum Source {
    File(PathBuf),
    Bytes(Vec<u8>),
    Dir,
    /// Kept as a link, so `node_modules/.bin` and friends stay intact.
    Link(PathBuf),
}

struct Entry {
    path: String,
    source: Source,
    mode: u32,
    size: u64,
}

/// Packs the release. `progress(done, total)` is called as input bytes are
/// compressed.
pub fn pack(
    deployable: &Deployable,
    manifest: &Manifest,
    unit: &str,
    mut progress: impl FnMut(u64, u64),
) -> Result<Bundle> {
    let selection = files::select(deployable)?;
    let replaced = selection.replaced.clone();
    let entries = collect(deployable, manifest, unit, selection)?;
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let files = entries
        .iter()
        .filter(|e| !matches!(e.source, Source::Dir | Source::Link(_)))
        .count();

    let mtime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut done = 0u64;
    progress(0, total);

    for entry in entries {
        let mut header = tar::Header::new_gnu();
        header.set_mode(entry.mode);
        header.set_mtime(mtime);
        header.set_uid(0);
        header.set_gid(0);
        header.set_size(entry.size);
        match entry.source {
            Source::Dir => {
                header.set_entry_type(tar::EntryType::Directory);
                tar.append_data(&mut header, &entry.path, io::empty())?;
            }
            Source::Link(target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                tar.append_link(&mut header, &entry.path, &target)
                    .with_context(|| format!("packing the link {}", entry.path))?;
            }
            Source::Bytes(bytes) => {
                header.set_entry_type(tar::EntryType::Regular);
                tar.append_data(&mut header, &entry.path, bytes.as_slice())?;
                done += entry.size;
                progress(done, total);
            }
            Source::File(local) => {
                header.set_entry_type(tar::EntryType::Regular);
                let file = std::fs::File::open(&local)
                    .with_context(|| format!("opening {}", local.display()))?;
                let reader = Counting {
                    inner: file,
                    on_read: |n| {
                        done += n;
                        progress(done, total);
                    },
                };
                tar.append_data(&mut header, &entry.path, reader)
                    .with_context(|| format!("packing {}", local.display()))?;
            }
        }
    }

    let data = tar
        .into_inner()?
        .finish()
        .context("finishing the archive")?;
    Ok(Bundle {
        data,
        files,
        input_bytes: total,
        replaced,
    })
}

/// Everything that goes into the archive, in order, with its mode.
fn collect(
    deployable: &Deployable,
    manifest: &Manifest,
    unit: &str,
    selection: Selection,
) -> Result<Vec<Entry>> {
    let service = &deployable.config.service;
    let unit_name = format!("{}.service", service.name);
    let manifest_json = serde_json::to_vec_pretty(manifest)?;

    let mut entries = Vec::new();
    if let (Some(binary), Some(name)) = (deployable.binary_path(), deployable.binary_name()) {
        let size = std::fs::metadata(&binary)
            .with_context(|| format!("the build did not produce {}", binary.display()))?
            .len();
        if let Some(item) = selection.items.get(&name) {
            bail!(
                "{name} would be in the release twice: pipeline.pack.binary, and {} from the {} folder",
                item.local.display(),
                item.source.name()
            );
        }
        entries.push(Entry {
            path: name,
            source: Source::File(binary),
            mode: 0o755,
            size,
        });
    }
    for reserved in [MANIFEST_FILE, unit_name.as_str()] {
        if let Some(item) = selection.items.get(reserved) {
            bail!(
                "{} would replace the release's own {reserved}; leave it out of pipeline.pack.files",
                item.local.display()
            );
        }
    }
    let shared: Vec<String> = service.shared.iter().map(|s| release_path(s)).collect();
    for path in selection.items.keys() {
        if let Some(folder) = shared
            .iter()
            .find(|folder| Path::new(path).starts_with(Path::new(folder.as_str())))
        {
            bail!(
                "{path} is packed, but service.shared links {folder} to shared/{folder} on the server; \
                 leave it out with pipeline.pack.files.exclude"
            );
        }
    }
    entries.push(Entry {
        path: MANIFEST_FILE.into(),
        size: manifest_json.len() as u64,
        source: Source::Bytes(manifest_json),
        mode: 0o644,
    });
    entries.push(Entry {
        path: unit_name,
        size: unit.len() as u64,
        source: Source::Bytes(unit.as_bytes().to_vec()),
        mode: 0o644,
    });

    for (path, item) in selection.items {
        entries.push(match item.kind {
            Kind::Dir => Entry {
                path,
                source: Source::Dir,
                mode: 0o755,
                size: 0,
            },
            Kind::Link(target) => Entry {
                path,
                source: Source::Link(target),
                mode: 0o777,
                size: 0,
            },
            Kind::File { size, mode } => Entry {
                path,
                source: Source::File(item.local),
                mode,
                size,
            },
        });
    }
    Ok(entries)
}

struct Counting<R, F> {
    inner: R,
    on_read: F,
}

impl<R: Read, F: FnMut(u64)> Read for Counting<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        (self.on_read)(n as u64);
        Ok(n)
    }
}

#[cfg(test)]
#[path = "bundle_test.rs"]
mod tests;
