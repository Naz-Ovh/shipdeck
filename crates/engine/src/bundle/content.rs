//! What a release holds, path by path, and the packing of it into one
//! `.tar.zst`: hashes, the receipt the server checks, and the fingerprint
//! that says whether anything changed.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::io::{self, Read};
use std::path::Path;
use std::thread;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use zstd::stream::write::Encoder;

use super::plan::{Entry, Plan, Source};

/// The file each release carries in `.shipdeck/`.
pub const MANIFEST_FILE: &str = ".shipdeck/release.json";

/// zstd level for the bundle. The slower levels only pay off on uploads
/// under about 100 KB/s.
const LEVEL: i32 = 9;

/// A packed release, ready to upload.
#[derive(Debug)]
pub struct Bundle {
    pub data: Vec<u8>,
    /// Files in the release, sent or reused.
    pub files: usize,
    /// Every path in the release, as it lands there, sent or reused.
    pub paths: Vec<String>,
    /// The release's size before compression, reused files included.
    pub input_bytes: u64,
    /// Files the live release already has: the server copies them from
    /// `current` instead of unpacking them.
    pub reused: usize,
    pub reused_bytes: u64,
    /// `sha256sum -c` input: `<hex>  ./<path>` lines.
    pub receipt: Vec<u8>,
    /// The reused files by the mode each gets: `./<path>` names, each
    /// ending in a NUL.
    pub reuse: BTreeMap<u32, Vec<u8>>,
}

impl Bundle {
    /// Files in the archive itself.
    #[must_use]
    pub fn sent(&self) -> usize {
        self.files - self.reused
    }
}

/// Hashes every file in `plan`, filling in each entry's SHA-256;
/// `progress(done, total)` follows the bytes read.
///
/// # Errors
/// Returns an error when a file cannot be read.
pub fn hash(plan: &mut Plan, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let total: u64 = plan
        .entries
        .iter()
        .filter(|entry| matches!(entry.source, Source::File(_)))
        .map(|entry| entry.size)
        .sum();
    let mut done = 0u64;
    progress(0, total);
    for entry in &mut plan.entries {
        entry.sha256 = match &entry.source {
            Source::File(local) => Some(sha256_file(local, |n| {
                done += n;
                progress(done, total);
            })?),
            Source::Bytes(bytes) => Some(sha256(bytes)),
            Source::Link(_) => None,
        };
    }
    Ok(())
}

/// `sha256:<hex>` over what makes the release what it is: each path with
/// its kind, a file's mode and SHA-256, a link's target. The settings file
/// is in the release, so a changed config is a changed release.
#[must_use]
pub fn fingerprint(plan: &Plan) -> String {
    let mut entries: Vec<&Entry> = plan
        .entries
        .iter()
        .filter(|entry| entry.path != MANIFEST_FILE)
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hash = Sha256::new();
    hash.update(b"shipdeck release content 2\0");
    for entry in entries {
        match &entry.source {
            Source::Link(target) => {
                field(&mut hash, b"l", entry.path.as_bytes());
                hash.update(target.as_os_str().as_encoded_bytes());
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
    format!("sha256:{}", hex(hash))
}

/// One NUL-ended record: its kind, then its text.
fn field(hash: &mut Sha256, kind: &[u8], text: &[u8]) {
    hash.update(kind);
    hash.update(text);
    hash.update(b"\0");
}

/// Whether `sha256sum` reads a path back as it was written. GNU and
/// busybox escape a name with a newline, a carriage return or a backslash
/// differently, so such a file is always sent and never listed.
#[must_use]
pub fn listable(path: &str) -> bool {
    !path.contains(['\n', '\r', '\\'])
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    base16ct::lower::encode_string(&Sha256::digest(bytes))
}

/// The digest so far, as lowercase hex.
pub(crate) fn hex(hash: Sha256) -> String {
    base16ct::lower::encode_string(&hash.finalize())
}

/// Reads `path` through once, without holding it in memory.
fn sha256_file(path: &Path, on_read: impl FnMut(u64)) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = Counting {
        inner: Hashing {
            inner: file,
            hash: Sha256::new(),
        },
        on_read,
    };
    io::copy(&mut reader, &mut io::sink())
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(hex(reader.inner.hash))
}

struct Hashing<R> {
    inner: R,
    hash: Sha256,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }
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

/// `sha256sum -c` input: `<hex>  ./<path>` for every file whose name it
/// reads back, sorted by path.
fn receipt<'a>(entries: impl Iterator<Item = &'a Entry>) -> Vec<u8> {
    let mut lines: Vec<(&str, &str)> = entries
        .filter(|entry| listable(&entry.path))
        .filter_map(|entry| Some((entry.path.as_str(), entry.sha256.as_deref()?)))
        .collect();
    lines.sort_unstable();
    let mut text = Vec::new();
    for (path, hash) in lines {
        text.extend_from_slice(format!("{hash}  ./{path}\n").as_bytes());
    }
    text
}

/// Packs the release. A file whose SHA-256 `on_server` has under its path
/// stays out of the archive, to be copied from the live release.
/// `progress(done, total)` is called as input bytes are compressed.
///
/// # Errors
/// Fails when a file changes while it is packed or the archive cannot be
/// written.
pub fn pack<S: BuildHasher>(
    plan: &Plan,
    on_server: &HashMap<String, String, S>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Bundle> {
    let entries: Vec<&Entry> = plan.entries.iter().collect();
    let is_reused = |entry: &Entry| {
        entry.is_candidate()
            && entry.sha256.is_some()
            && on_server.get(&entry.path) == entry.sha256.as_ref()
    };
    let paths = entries.iter().map(|entry| entry.path.clone()).collect();
    let files = entries
        .iter()
        .filter(|entry| !matches!(entry.source, Source::Link(_)))
        .count();
    let input_bytes: u64 = entries.iter().map(|entry| entry.size).sum();
    let (reuse, reused, reused_bytes) = reuse_lists(&entries, &is_reused);
    let receipt = receipt(entries.iter().copied());
    let total = input_bytes.saturating_sub(reused_bytes);
    let mtime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let data = archive(&entries, &is_reused, mtime, total, &mut progress)?;
    Ok(Bundle {
        data,
        files,
        paths,
        input_bytes,
        reused,
        reused_bytes,
        receipt,
        reuse,
    })
}

/// What the live release already holds, per mode, as the archive wants it.
fn reuse_lists(
    entries: &[&Entry],
    is_reused: &impl Fn(&Entry) -> bool,
) -> (BTreeMap<u32, Vec<u8>>, usize, u64) {
    let mut reuse: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let (mut count, mut bytes) = (0, 0);
    for entry in entries.iter().filter(|entry| is_reused(entry)) {
        let list = reuse.entry(entry.mode).or_default();
        list.extend_from_slice(b"./");
        list.extend_from_slice(entry.path.as_bytes());
        list.push(0);
        count += 1;
        bytes += entry.size;
    }
    (reuse, count, bytes)
}

/// Writes the tar and compresses it, calling `progress` as bytes go in.
fn archive(
    entries: &[&Entry],
    is_reused: &impl Fn(&Entry) -> bool,
    mtime: u64,
    total: u64,
    progress: &mut impl FnMut(u64, u64),
) -> Result<Vec<u8>> {
    let mut tar = tar::Builder::new(encoder()?);
    let mut done = 0u64;
    progress(0, total);

    for entry in entries.iter().filter(|entry| !is_reused(entry)) {
        let mut header = tar::Header::new_gnu();
        header.set_mode(entry.mode);
        header.set_mtime(mtime);
        header.set_uid(0);
        header.set_gid(0);
        header.set_size(entry.size);
        match &entry.source {
            Source::Link(target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                tar.append_link(&mut header, &entry.path, target)
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
                let file = std::fs::File::open(local)
                    .with_context(|| format!("opening {}", local.display()))?;
                let mut reader = Counting {
                    inner: Hashing {
                        inner: file,
                        hash: Sha256::new(),
                    },
                    on_read: |n| {
                        done += n;
                        progress(done, total);
                    },
                };
                tar.append_data(&mut header, &entry.path, &mut reader)
                    .with_context(|| format!("packing {}", local.display()))?;
                let packed = hex(reader.inner.hash);
                if Some(&packed) != entry.sha256.as_ref() {
                    bail!(
                        "{} changed while the release was packed; deploy again once nothing writes to it",
                        local.display()
                    );
                }
            }
        }
    }

    tar.into_inner()?.finish().context("finishing the archive")
}

/// A zstd stream on every core, with a checksum so `zstd -d` on the server
/// refuses a damaged upload.
fn encoder() -> Result<Encoder<'static, Vec<u8>>> {
    let mut encoder = Encoder::new(Vec::new(), LEVEL)?;
    encoder.include_checksum(true)?;
    let workers = thread::available_parallelism().map_or(1, std::num::NonZero::get);
    encoder.multithread(u32::try_from(workers).unwrap_or(1))?;
    Ok(encoder)
}

impl Entry {
    /// A regular file whose name `sha256sum` reads back.
    fn is_candidate(&self) -> bool {
        matches!(self.source, Source::File(_) | Source::Bytes(_)) && listable(&self.path)
    }
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
