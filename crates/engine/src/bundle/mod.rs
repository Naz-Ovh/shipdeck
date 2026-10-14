//! Packing a release into one `.tar.gz` in memory: the binary, the build's
//! `build.files`, the deployable's `bundle` files, `release.json` and the
//! systemd unit.

pub mod unit;

use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::config::{Deployable, bundle_path};
use crate::release::{MANIFEST_FILE, Manifest};

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
    let entries = collect(deployable, manifest, unit)?;
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
    })
}

/// Everything that goes into the archive, in order, with its mode.
fn collect(deployable: &Deployable, manifest: &Manifest, unit: &str) -> Result<Vec<Entry>> {
    let unit_name = format!("{}.service", deployable.config.remote.service);
    let manifest_json = serde_json::to_vec_pretty(manifest)?;

    let mut entries = Vec::new();
    if let (Some(binary), Some(name)) = (deployable.binary_path(), deployable.binary_name()) {
        let size = std::fs::metadata(&binary)
            .with_context(|| format!("the build did not produce {}", binary.display()))?
            .len();
        entries.push(Entry {
            path: name,
            source: Source::File(binary),
            mode: 0o755,
            size,
        });
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

    // Entries were validated when deploy.yaml loaded and checked before;
    // this catches a file that vanished since.
    deployable.check_files()?;
    let build_dir = deployable.build_dir();
    for item in &deployable.config.build.files {
        add(&build_dir.join(item), bundle_path(item), &mut entries)?;
    }
    deployable.check_bundle()?;
    for item in &deployable.config.bundle {
        add(
            &deployable.folder.join(item),
            bundle_path(item),
            &mut entries,
        )?;
    }
    Ok(entries)
}

/// Adds a listed entry. A link the entry itself names is followed, so
/// `dist -> build/out` ships the folder; links inside a folder stay links,
/// so `node_modules/.bin` keeps working.
fn add(local: &Path, archive_path: String, entries: &mut Vec<Entry>) -> Result<()> {
    let meta = std::fs::metadata(local).with_context(|| format!("reading {}", local.display()))?;
    add_found(local, archive_path, meta, entries)
}

fn add_found(
    local: &Path,
    archive_path: String,
    meta: std::fs::Metadata,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(local)
            .with_context(|| format!("reading the link {}", local.display()))?;
        entries.push(Entry {
            path: archive_path,
            source: Source::Link(target),
            mode: 0o777,
            size: 0,
        });
    } else if meta.is_dir() {
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
            let meta = std::fs::symlink_metadata(&child)
                .with_context(|| format!("reading {}", child.display()))?;
            add_found(&child, format!("{archive_path}/{name}"), meta, entries)?;
        }
    } else if meta.is_file() {
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
    // Sockets and pipes are skipped; they mean nothing on another machine.
    Ok(())
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
