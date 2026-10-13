//! Packing a release into one `.tar.gz` in memory: the binary, the
//! deployable's `bundle` files, `release.json` and the systemd unit.

pub mod unit;

use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::config::Deployable;
use crate::release::Manifest;

pub const MANIFEST_NAME: &str = "release.json";

/// A packed release, ready to upload.
#[derive(Debug)]
pub struct Bundle {
    pub data: Vec<u8>,
    pub files: usize,
    /// Size before compression.
    pub input_bytes: u64,
}

enum Source {
    File(PathBuf),
    Bytes(Vec<u8>),
    Dir,
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
    let entries = collect(deployable, manifest, unit)?;
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let files = entries
        .iter()
        .filter(|e| !matches!(e.source, Source::Dir))
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
    })
}

/// Everything that goes into the archive, in order, with its mode.
fn collect(deployable: &Deployable, manifest: &Manifest, unit: &str) -> Result<Vec<Entry>> {
    let binary_name = deployable.binary_name();
    let unit_name = format!("{}.service", deployable.config.remote.service);
    let reserved = [binary_name.as_str(), MANIFEST_NAME, unit_name.as_str()];

    let binary = deployable.binary_path();
    let binary_size = std::fs::metadata(&binary)
        .with_context(|| format!("the build did not produce {}", binary.display()))?
        .len();
    let manifest_json = serde_json::to_vec_pretty(manifest)?;

    let mut entries = vec![
        Entry {
            path: binary_name.clone(),
            source: Source::File(binary),
            mode: 0o755,
            size: binary_size,
        },
        Entry {
            path: MANIFEST_NAME.into(),
            size: manifest_json.len() as u64,
            source: Source::Bytes(manifest_json),
            mode: 0o644,
        },
        Entry {
            path: unit_name.clone(),
            size: unit.len() as u64,
            source: Source::Bytes(unit.as_bytes().to_vec()),
            mode: 0o644,
        },
    ];

    for item in &deployable.config.bundle {
        let archive_path = normalize(item);
        if reserved.contains(&archive_path.as_str()) {
            bail!("bundle entry {item:?} would overwrite the release's own {archive_path}");
        }
        let local = deployable.folder.join(item);
        if !local.exists() {
            bail!(
                "bundle entry {item:?} is missing: {} does not exist",
                local.display()
            );
        }
        add(&local, archive_path, &mut entries)?;
    }
    Ok(entries)
}

fn add(local: &Path, archive_path: String, entries: &mut Vec<Entry>) -> Result<()> {
    let meta = std::fs::metadata(local).with_context(|| format!("reading {}", local.display()))?;
    if meta.is_dir() {
        entries.push(Entry {
            path: archive_path.clone(),
            source: Source::Dir,
            mode: 0o755,
            size: 0,
        });
        let mut children: Vec<PathBuf> = std::fs::read_dir(local)
            .with_context(|| format!("reading {}", local.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        children.sort();
        for child in children {
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            add(&child, format!("{archive_path}/{name}"), entries)?;
        }
    } else {
        let name = archive_path.rsplit('/').next().unwrap_or(&archive_path);
        // Secrets stay readable by their owner only, whatever the local mode is.
        let mode = if name.starts_with(".env") {
            0o600
        } else {
            meta.permissions().mode() & 0o777
        };
        entries.push(Entry {
            path: archive_path,
            source: Source::File(local.to_path_buf()),
            mode,
            size: meta.len(),
        });
    }
    Ok(())
}

/// `./files/` → `files`
fn normalize(item: &str) -> String {
    let trimmed = item.trim_end_matches('/');
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    trimmed.to_string()
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
