//! Packing a release into one `.tar.zst` in memory: the binary, the files
//! `pipeline.pack.files` picks, `release.json` and the service's own file
//! (the systemd unit or the PM2 app file). Files the live release already
//! has stay out; the server copies them from it.

mod apps;
pub mod content;
pub mod files;
pub mod pm2;
pub mod unit;

pub use apps::{fingerprint_monorepo, gather_monorepo};
pub use content::{Contents, listable};

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use zstd::stream::write::Encoder;

use crate::config::{Deployable, Manager, Pack, Service, is_ecosystem_file, release_path};
use crate::release::{MANIFEST_FILE, Manifest, ReleaseName};
use content::Hashing;
use files::{Kind, Selection};

/// zstd level for the bundle. On a 25 MB Rust binary, 9 packs to 8.6 MB in
/// 0.6s where 19 packs to 7.6 MB in 10s: the slower levels only pay off on
/// uploads under about 100 KB/s.
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
    /// `sha256sum -c` input for every file of the release whose name it
    /// reads back: `<hex>  ./<path>` lines.
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

#[derive(Debug, Clone)]
enum Source {
    File(PathBuf),
    Bytes(Vec<u8>),
    Dir,
    /// Kept as a link, so `node_modules/.bin` and friends stay intact.
    Link(PathBuf),
}

#[derive(Debug, Clone)]
struct Entry {
    path: String,
    source: Source,
    mode: u32,
    size: u64,
    /// For a file, once hashed.
    sha256: Option<String>,
}

/// The file shipdeck writes into a release for its manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFile {
    /// Its path in the release: `<service>.service` or `<service>.pm2.json`.
    pub name: String,
    pub text: String,
}

/// Settings needed to render one process in a release.
pub struct ProcessSpec<'a> {
    /// The displayed deploy name.
    pub name: &'a str,
    /// The process settings.
    pub service: &'a Service,
    /// The packed binary's file name, if one is configured.
    pub binary_name: Option<&'a str>,
    /// The user that runs the process.
    pub user: &'a str,
    /// A custom systemd template, when present.
    pub template: Option<&'a Path>,
    /// The process folder within the release.
    pub release_dir: &'a str,
}

/// One app's local files and its destination in a shared release.
pub struct AppSource<'a> {
    /// The app's project folder.
    pub project: &'a Path,
    /// The shared deployment folder.
    pub deploy_folder: &'a Path,
    /// The app's packing settings.
    pub pack: &'a Pack,
    /// The app's service settings.
    pub service: &'a Service,
    /// The app's generated process manager file, if any.
    pub service_file: Option<&'a ServiceFile>,
    /// Its folder in the release; an empty path means the release root.
    pub release_dir: &'a str,
    /// Absolute paths of app config files that must not be shipped.
    pub excluded_configs: &'a [PathBuf],
}

/// Renders the release's service file; `None` when `pm2.ecosystem` names
/// the user's own. `start_path` is where preflight found the start program.
///
/// # Errors
///
/// Fails when the unit or PM2 app file cannot be rendered.
pub fn service_file(
    deployable: &Deployable,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<Option<ServiceFile>> {
    service_file_in(deployable, release, start_path, "")
}

/// Renders a process file for a folder within the shared release.
///
/// # Errors
/// Returns an error when the folder or process file is invalid.
pub fn service_file_in(
    deployable: &Deployable,
    release: &ReleaseName,
    start_path: Option<&str>,
    release_dir: &str,
) -> Result<Option<ServiceFile>> {
    let binary_name = deployable.binary_name();
    let template = deployable.unit_template_path();
    let spec = ProcessSpec {
        name: &deployable.config.name,
        service: &deployable.config.service,
        binary_name: binary_name.as_deref(),
        user: deployable.run_as(),
        template: Some(&template),
        release_dir,
    };
    service_file_for(&spec, release, start_path)
}

/// Renders a process file from its settings and release folder.
///
/// # Errors
/// Returns an error when the folder or process file is invalid.
pub fn service_file_for(
    spec: &ProcessSpec<'_>,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<Option<ServiceFile>> {
    let dir = Path::new(spec.release_dir);
    if spec.release_dir.contains(['\\', '\n', '\r'])
        || dir.is_absolute()
        || dir
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        bail!(
            "release folder {:?} must be a relative path",
            spec.release_dir
        );
    }
    let service = spec.service;
    let Some(name) = service.own_file() else {
        return Ok(None);
    };
    let text = match service.manager {
        Manager::Systemd => unit::render_spec(spec, release, start_path)?,
        Manager::Pm2 => pm2::render_spec(spec, start_path)?,
    };
    Ok(Some(ServiceFile { name, text }))
}

/// What the service runs, in one line for the screen and the log.
#[must_use]
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

/// Picks what goes into the release and hashes every file in it.
/// `progress(done, total)` follows the bytes read.
///
/// # Errors
///
/// Fails when the file selection is invalid, or a file cannot be read or
/// changes while it is hashed.
pub fn gather(
    deployable: &Deployable,
    own: Option<&ServiceFile>,
    progress: impl FnMut(u64, u64),
) -> Result<Contents> {
    match own {
        Some(file) => gather_services(deployable, std::slice::from_ref(&file), progress),
        None => gather_services(deployable, &[], progress),
    }
}

/// Picks release files and all generated process manager files.
///
/// # Errors
/// Returns an error when files collide or cannot be read.
pub fn gather_services(
    deployable: &Deployable,
    own: &[&ServiceFile],
    progress: impl FnMut(u64, u64),
) -> Result<Contents> {
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
    let binary_path = deployable.binary_path();
    let binary_name = deployable.binary_name();
    let binary = binary_path.as_deref().zip(binary_name.as_deref());
    let mut entries = collect(&deployable.config.service, binary, own, selection, true)?;
    content::hash_all(&mut entries, progress)?;
    Ok(Contents {
        entries,
        own: own.iter().map(|file| file.name.clone()).collect(),
        replaced,
        unused_ecosystem,
    })
}

/// Picks files for several apps and hashes them as one release.
///
/// # Errors
/// Returns an error when a path is unsafe, files collide, or a file cannot be read.
pub fn gather_apps(
    apps: &[AppSource<'_>],
    metadata: &[u8],
    progress: impl FnMut(u64, u64),
) -> Result<Contents> {
    let own_names: Vec<&str> = apps
        .iter()
        .filter_map(|app| app.service_file.map(|file| file.name.as_str()))
        .collect();
    check_process_names(&own_names)?;
    let mut entries = BTreeMap::new();
    let mut own = Vec::new();
    let mut replaced = Vec::new();
    let mut unused_ecosystem = Vec::new();
    insert_entry(
        &mut entries,
        Entry {
            path: crate::config::monorepo::RELEASE_METADATA.into(),
            source: Source::Bytes(metadata.to_vec()),
            mode: 0o600,
            size: u64::try_from(metadata.len())?,
            sha256: None,
        },
    )?;
    for app in apps {
        let dir = Path::new(app.release_dir);
        if app.release_dir.contains(['\\', '\n', '\r'])
            || dir.is_absolute()
            || dir
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!(
                "release folder {:?} must be a relative path",
                app.release_dir
            );
        }
        let selection = files::select_paths(
            app.project,
            app.deploy_folder,
            &app.pack.files,
            app.excluded_configs,
        )?;
        let service = app.service;
        if service.manager == Manager::Pm2 && service.pm2.ecosystem.is_none() {
            unused_ecosystem.extend(
                selection
                    .items
                    .keys()
                    .filter(|path| path.starts_with("ecosystem.") && is_ecosystem_file(path))
                    .map(|path| release_destination(app.release_dir, path)),
            );
        }
        replaced.extend(
            selection
                .replaced
                .iter()
                .map(|path| release_destination(app.release_dir, path)),
        );
        if !app.release_dir.is_empty() {
            insert_entry(
                &mut entries,
                Entry {
                    path: app.release_dir.to_string(),
                    source: Source::Dir,
                    mode: 0o755,
                    size: 0,
                    sha256: None,
                },
            )?;
        }
        if let Some(file) = app.service_file {
            own.push(file.name.clone());
            insert_entry(
                &mut entries,
                Entry {
                    path: file.name.clone(),
                    size: u64::try_from(file.text.len())?,
                    source: Source::Bytes(file.text.as_bytes().to_vec()),
                    mode: 0o644,
                    sha256: None,
                },
            )?;
        }
        let binary = app_binary(app.project, app.pack.binary.as_deref())?;
        let binary = binary
            .as_ref()
            .map(|(path, name)| (path.as_path(), name.as_str()));
        for mut entry in collect(service, binary, &[], selection, app.release_dir.is_empty())? {
            entry.path = release_destination(app.release_dir, &entry.path);
            if entry.path == MANIFEST_FILE || own_names.contains(&entry.path.as_str()) {
                bail!("{} is reserved for the release", entry.path);
            }
            insert_entry(&mut entries, entry)?;
        }
    }
    check_parents(&entries)?;
    let mut entries: Vec<Entry> = entries.into_values().collect();
    content::hash_all(&mut entries, progress)?;
    Ok(Contents {
        entries,
        own,
        replaced,
        unused_ecosystem,
    })
}

/// Checks generated process file names before adding them to a release.
fn check_process_names(names: &[&str]) -> Result<()> {
    for name in names {
        if name.starts_with('.')
            || !(name.ends_with(".service") || name.ends_with(".pm2.json"))
            || !crate::config::is_unit_name(name)
        {
            bail!("{name:?} is not a safe process file name");
        }
    }
    Ok(())
}

/// Refuses a release path whose parent is a file or a link.
fn check_parents(entries: &BTreeMap<String, Entry>) -> Result<()> {
    for path in entries.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(folder) = parent {
            if let Some(folder) = folder.to_str()
                && entries
                    .get(folder)
                    .is_some_and(|entry| !matches!(entry.source, Source::Dir))
            {
                bail!("{path} is inside a file or link {folder}");
            }
            parent = folder.parent();
        }
    }
    Ok(())
}

/// Resolves an app binary without letting its path escape the project.
fn app_binary(project: &Path, binary: Option<&str>) -> Result<Option<(PathBuf, String)>> {
    let Some(binary) = binary else {
        return Ok(None);
    };
    let path = Path::new(binary);
    if binary.is_empty()
        || path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("binary {binary:?} must be inside its project");
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("binary name is not UTF-8")?
        .to_string();
    let root = project
        .canonicalize()
        .with_context(|| format!("opening {}", project.display()))?;
    let path = root
        .join(path)
        .canonicalize()
        .with_context(|| format!("opening binary {binary}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        bail!("binary {binary:?} must be a file inside its project");
    }
    Ok(Some((path, name)))
}

/// Joins a release folder to a path selected inside it.
fn release_destination(folder: &str, path: &str) -> String {
    if folder.is_empty() {
        path.to_string()
    } else {
        format!("{folder}/{path}")
    }
}

/// Adds an entry, merging folders and rejecting all other collisions.
fn insert_entry(entries: &mut BTreeMap<String, Entry>, entry: Entry) -> Result<()> {
    if let Some(old) = entries.get(&entry.path) {
        if matches!((&old.source, &entry.source), (Source::Dir, Source::Dir)) {
            return Ok(());
        }
        bail!("{} would be in the release more than once", entry.path);
    }
    entries.insert(entry.path.clone(), entry);
    Ok(())
}

/// Packs the release with `manifest` as its `release.json`. A file whose
/// SHA-256 `on_server` has under its path stays out of the archive, to be
/// copied from the live release. `progress(done, total)` is called as
/// input bytes are compressed.
///
/// # Errors
///
/// Fails when the manifest cannot be serialized, a file changes while it
/// is packed, or the archive cannot be written.
pub fn pack<S: BuildHasher>(
    contents: &Contents,
    manifest: &Manifest,
    on_server: &HashMap<String, String, S>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Bundle> {
    let json = serde_json::to_vec_pretty(manifest)?;
    let manifest = Entry {
        path: MANIFEST_FILE.into(),
        size: json.len() as u64,
        sha256: Some(content::sha256(&json)),
        source: Source::Bytes(json),
        mode: 0o644,
    };
    let entries: Vec<&Entry> = std::iter::once(&manifest)
        .chain(&contents.entries)
        .collect();
    let is_reused = |entry: &Entry| {
        entry.is_candidate()
            && entry.sha256.is_some()
            && on_server.get(&entry.path) == entry.sha256.as_ref()
    };
    let paths = entries.iter().map(|e| e.path.clone()).collect();
    let files = entries
        .iter()
        .filter(|e| !matches!(e.source, Source::Dir | Source::Link(_)))
        .count();
    let input_bytes: u64 = entries.iter().map(|e| e.size).sum();
    let (reuse, reused, reused_bytes) = reuse_lists(&entries, &is_reused);
    let receipt = content::receipt(entries.iter().copied());
    let total = input_bytes - reused_bytes;
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
    for entry in entries.iter().filter(|e| is_reused(e)) {
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

    for entry in entries.iter().filter(|e| !is_reused(e)) {
        let mut header = tar::Header::new_gnu();
        header.set_mode(entry.mode);
        header.set_mtime(mtime);
        header.set_uid(0);
        header.set_gid(0);
        header.set_size(entry.size);
        match &entry.source {
            Source::Dir => {
                header.set_entry_type(tar::EntryType::Directory);
                tar.append_data(&mut header, &entry.path, io::empty())?;
            }
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
                let packed = content::hex(reader.inner.hash);
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
/// refuses a damaged upload, as gzip's CRC did.
fn encoder() -> Result<Encoder<'static, Vec<u8>>> {
    let mut encoder = Encoder::new(Vec::new(), LEVEL)?;
    encoder.include_checksum(true)?;
    let workers = thread::available_parallelism().map_or(1, std::num::NonZero::get);
    encoder.multithread(u32::try_from(workers).unwrap_or(1))?;
    Ok(encoder)
}

/// Everything that goes into the release but `release.json`, in order,
/// with its mode.
fn collect(
    service: &Service,
    binary: Option<(&Path, &str)>,
    own: &[&ServiceFile],
    selection: Selection,
    root: bool,
) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    if let Some((binary, name)) = binary {
        let size = std::fs::metadata(binary)
            .with_context(|| format!("the build did not produce {}", binary.display()))?
            .len();
        if let Some(item) = selection.items.get(name) {
            bail!(
                "{name} would be in the release twice: pipeline.pack.binary, and {} from the {} folder",
                item.local.display(),
                item.source.name()
            );
        }
        entries.push(Entry {
            path: name.to_string(),
            source: Source::File(binary.to_path_buf()),
            mode: 0o755,
            size,
            sha256: None,
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
    for reserved in root
        .then_some(MANIFEST_FILE)
        .into_iter()
        .chain(own.iter().map(|file| file.name.as_str()))
    {
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
    for (index, file) in own.iter().enumerate() {
        if file.name == MANIFEST_FILE
            || own[..index].iter().any(|other| other.name == file.name)
            || entries.iter().any(|entry| entry.path == file.name)
        {
            bail!("{} would be in the release more than once", file.name);
        }
        entries.push(Entry {
            path: file.name.clone(),
            size: file.text.len() as u64,
            source: Source::Bytes(file.text.as_bytes().to_vec()),
            mode: 0o644,
            sha256: None,
        });
    }

    for (path, item) in selection.items {
        entries.push(match item.kind {
            Kind::Dir => Entry {
                path,
                source: Source::Dir,
                mode: 0o755,
                size: 0,
                sha256: None,
            },
            Kind::Link(target) => Entry {
                path,
                source: Source::Link(target),
                mode: 0o777,
                size: 0,
                sha256: None,
            },
            Kind::File { size, mode } => Entry {
                path,
                source: Source::File(item.local),
                mode,
                size,
                sha256: None,
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

#[cfg(test)]
#[path = "multiple_test.rs"]
mod multiple_tests;
