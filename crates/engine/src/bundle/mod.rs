//! Packing a release into one `.tar.zst` in memory: the binary, the files
//! `pipeline.pack.files` picks, `release.json` and the service's own file
//! (the systemd unit or the PM2 app file).

pub mod files;
pub mod pm2;
pub mod unit;

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, bail};
use zstd::stream::write::Encoder;

use crate::config::{Deployable, Manager, is_ecosystem_file, release_path};
use crate::release::{MANIFEST_FILE, Manifest, ReleaseName};
use files::{Kind, Selection};

/// zstd level for the bundle. On a 25 MB Rust binary, 9 packs to 8.6 MB in
/// 0.6s where 19 packs to 7.6 MB in 10s: the slower levels only pay off on
/// uploads under about 100 KB/s.
const LEVEL: i32 = 9;

/// A packed release, ready to upload.
#[derive(Debug)]
pub struct Bundle {
    pub data: Vec<u8>,
    pub files: usize,
    /// Size before compression.
    pub input_bytes: u64,
    /// Project files a file from the deploy folder replaced.
    pub replaced: Vec<String>,
    /// Ecosystem files in the release PM2 never reads, because shipdeck
    /// writes the app file from `pm2.options`.
    pub unused_ecosystem: Vec<String>,
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

/// The file shipdeck writes into a release for its manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFile {
    /// Its path in the release: `<service>.service` or `<service>.pm2.json`.
    pub name: String,
    pub text: String,
}

/// Renders the release's service file; `None` when `pm2.ecosystem` names
/// the user's own. `start_path` is where preflight found the start program.
pub fn service_file(
    deployable: &Deployable,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<Option<ServiceFile>> {
    let service = &deployable.config.service;
    let Some(name) = service.own_file() else {
        return Ok(None);
    };
    let text = match service.manager {
        Manager::Systemd => unit::render_for(deployable, release, start_path)?,
        Manager::Pm2 => pm2::render(deployable, start_path)?,
    };
    Ok(Some(ServiceFile { name, text }))
}

/// What the service runs, in one line for the screen and the log.
pub fn start_line(deployable: &Deployable, start_path: Option<&str>) -> String {
    let service = &deployable.config.service;
    match (service.manager, &service.pm2.ecosystem) {
        (Manager::Systemd, _) => unit::exec_start(deployable, start_path),
        (Manager::Pm2, Some(file)) => format!("pm2 app {} from {file}", service.pm2_app()),
        (Manager::Pm2, None) => format!(
            "pm2 app {}: {}",
            service.pm2_app(),
            unit::exec_start(deployable, start_path)
        ),
    }
}

/// Packs the release. `progress(done, total)` is called as input bytes are
/// compressed.
pub fn pack(
    deployable: &Deployable,
    manifest: &Manifest,
    own: Option<&ServiceFile>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Bundle> {
    let selection = files::select(deployable)?;
    let replaced = selection.replaced.clone();
    let service = &deployable.config.service;
    let unused_ecosystem = if service.manager == Manager::Pm2 && service.pm2.ecosystem.is_none() {
        selection
            .items
            .keys()
            .filter(|path| path.starts_with("ecosystem.") && is_ecosystem_file(path))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let entries = collect(deployable, manifest, own, selection)?;
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let files = entries
        .iter()
        .filter(|e| !matches!(e.source, Source::Dir | Source::Link(_)))
        .count();

    let mtime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut tar = tar::Builder::new(encoder()?);
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
        unused_ecosystem,
    })
}

/// A zstd stream on every core, with a checksum so `zstd -d` on the server
/// refuses a damaged upload, as gzip's CRC did.
fn encoder() -> Result<Encoder<'static, Vec<u8>>> {
    let mut encoder = Encoder::new(Vec::new(), LEVEL)?;
    encoder.include_checksum(true)?;
    let workers = thread::available_parallelism().map_or(1, |n| n.get());
    encoder.multithread(u32::try_from(workers).unwrap_or(1))?;
    Ok(encoder)
}

/// Everything that goes into the archive, in order, with its mode.
fn collect(
    deployable: &Deployable,
    manifest: &Manifest,
    own: Option<&ServiceFile>,
    selection: Selection,
) -> Result<Vec<Entry>> {
    let service = &deployable.config.service;
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
    if let Some(file) = &service.pm2.ecosystem {
        let file = release_path(file);
        if !selection.items.contains_key(&file) {
            bail!(
                "pm2.ecosystem names {file}, but the release has no {file}; \
                 add it to pipeline.pack.files.include"
            );
        }
    }
    let own_name = own.map(|file| file.name.as_str());
    for reserved in [Some(MANIFEST_FILE), own_name].into_iter().flatten() {
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
    if let Some(own) = own {
        entries.push(Entry {
            path: own.name.clone(),
            size: own.text.len() as u64,
            source: Source::Bytes(own.text.as_bytes().to_vec()),
            mode: 0o644,
        });
    }

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
