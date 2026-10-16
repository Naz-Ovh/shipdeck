//! What a release holds, file by file: each file's SHA-256, which decides
//! what the live release already has; the receipt the server checks the
//! unpacked release against; and the fingerprint that tells whether
//! anything changed at all.

use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::DateTime;
use sha2::{Digest, Sha256};

use super::{Counting, Entry, ServiceFile, Source, service_file};
use crate::config::Deployable;
use crate::release::ReleaseName;

/// Everything a release holds except its `release.json`, each file with
/// its SHA-256.
#[derive(Debug, Clone)]
pub struct Contents {
    pub(super) entries: Vec<Entry>,
    /// The file shipdeck writes for the service, if it writes one.
    pub(super) own: Option<String>,
    /// Project files a file from the deploy folder replaced.
    pub replaced: Vec<String>,
    /// Ecosystem files in the release PM2 never reads, because shipdeck
    /// writes the app file from `pm2.options`.
    pub unused_ecosystem: Vec<String>,
}

impl Contents {
    /// The files the live release may already have: every one from the
    /// project or the deploy folder whose name `sha256sum` reads back.
    /// shipdeck's own files change with each release and are always sent.
    pub fn candidates(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.is_candidate())
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// `sha256:<hex>` over what makes the release what it is: each path with
    /// its kind, a file's mode and SHA-256, a link's target; the service file
    /// rendered without the release's name, so a new id, version or build
    /// time leave it alone; and `deploy.yaml`. Two releases with the same
    /// fingerprint run the same files the same way.
    pub fn fingerprint(&self, deployable: &Deployable, start_path: Option<&str>) -> Result<String> {
        let own = service_file(deployable, &unnamed(), start_path)?;
        Ok(self.fingerprint_with(own.as_ref(), &deployable.yaml))
    }

    /// The fingerprint, with the service file already rendered.
    pub(super) fn fingerprint_with(&self, own: Option<&ServiceFile>, yaml: &str) -> String {
        let mut entries: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| Some(&entry.path) != self.own.as_ref())
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        // Paths cannot hold a NUL, so NUL-ended fields never run together.
        let mut hash = Sha256::new();
        hash.update(b"shipdeck release content 1\0");
        for entry in entries {
            match &entry.source {
                Source::Dir => field(&mut hash, b"d", entry.path.as_bytes()),
                Source::Link(target) => {
                    field(&mut hash, b"l", entry.path.as_bytes());
                    hash.update(target.as_os_str().as_bytes());
                    hash.update(b"\0");
                }
                Source::File(_) | Source::Bytes(_) => {
                    field(&mut hash, b"f", entry.path.as_bytes());
                    hash.update(format!("{:o}\0", entry.mode));
                    hash.update(entry.sha256.as_deref().unwrap_or_default());
                    hash.update(b"\0");
                }
            }
        }
        if let Some(own) = own {
            field(&mut hash, b"s", own.name.as_bytes());
            hash.update(sha256(own.text.as_bytes()));
            hash.update(b"\0");
        }
        field(&mut hash, b"y", sha256(yaml.as_bytes()).as_bytes());
        format!("sha256:{:x}", hash.finalize())
    }
}

/// One NUL-ended record: its kind, then its text.
fn field(hash: &mut Sha256, kind: &[u8], text: &[u8]) {
    hash.update(kind);
    hash.update(text);
    hash.update(b"\0");
}

/// The release name a fingerprint renders the service file with.
fn unnamed() -> ReleaseName {
    ReleaseName::new("0.0.0", "unnamed", false, DateTime::UNIX_EPOCH)
}

/// Whether `sha256sum` reads a path back as it was written. GNU and
/// busybox escape a name with a newline, a carriage return or a backslash
/// differently, so such a file is always sent and never listed.
pub fn listable(path: &str) -> bool {
    !path.contains(['\n', '\r', '\\'])
}

impl Entry {
    /// From the project or the deploy folder, under a name `sha256sum`
    /// reads back.
    pub(super) fn is_candidate(&self) -> bool {
        matches!(self.source, Source::File(_)) && listable(&self.path)
    }
}

/// Hashes every file in `entries`; `progress(done, total)` follows the
/// bytes read.
pub(super) fn hash_all(entries: &mut [Entry], mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let total: u64 = entries
        .iter()
        .filter(|e| matches!(e.source, Source::File(_)))
        .map(|e| e.size)
        .sum();
    let mut done = 0u64;
    progress(0, total);
    for entry in entries {
        entry.sha256 = match &entry.source {
            Source::File(local) => Some(sha256_file(local, |n| {
                done += n;
                progress(done, total);
            })?),
            Source::Bytes(bytes) => Some(sha256(bytes)),
            Source::Dir | Source::Link(_) => None,
        };
    }
    Ok(())
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Reads `path` through once, without holding it in memory.
fn sha256_file(path: &Path, on_read: impl FnMut(u64)) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hash = Sha256::new();
    io::copy(
        &mut Counting {
            inner: file,
            on_read,
        },
        &mut hash,
    )
    .with_context(|| format!("reading {}", path.display()))?;
    Ok(format!("{:x}", hash.finalize()))
}

/// Hashes what is read through it, so packing can tell a file changed
/// after it was hashed.
pub(super) struct Hashing<R> {
    pub inner: R,
    pub hash: Sha256,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }
}

/// `sha256sum -c` input: `<hex>  ./<path>` for every file in `entries`
/// whose name it reads back, sorted by path.
pub(super) fn receipt<'a>(entries: impl Iterator<Item = &'a Entry>) -> Vec<u8> {
    let mut lines: Vec<(&str, &str)> = entries
        .filter(|entry| listable(&entry.path))
        .filter_map(|entry| Some((entry.path.as_str(), entry.sha256.as_deref()?)))
        .collect();
    lines.sort();
    let mut text = Vec::new();
    for (path, hash) in lines {
        text.extend_from_slice(format!("{hash}  ./{path}\n").as_bytes());
    }
    text
}
