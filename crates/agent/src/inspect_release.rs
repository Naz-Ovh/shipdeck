//! Looking inside a release folder or its backup, without changing it.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{
    AgentDropRequest, AgentFileInfo, AgentFileKind, AgentOk, AgentReleaseContents,
    AgentReleaseContentsRequest, AgentReleaseRead, AgentReleaseReadRequest,
};

/// How many paths one listing may hold.
const MAX_ENTRIES: u64 = 1_000_000;
/// How much of one file is read.
const MAX_READ: u64 = 512 * 1024;

/// Lists a release folder or its backup.
pub(crate) async fn contents(request: AgentReleaseContentsRequest) -> Result<AgentReleaseContents> {
    tokio::task::spawn_blocking(move || contents_sync(&request)).await?
}

/// Reads the start of one file from a release folder or its backup.
pub(crate) async fn read(request: AgentReleaseReadRequest) -> Result<AgentReleaseRead> {
    tokio::task::spawn_blocking(move || read_sync(&request)).await?
}

/// Removes one opened release or one backup.
pub(crate) async fn drop_one(request: AgentDropRequest) -> Result<AgentOk> {
    tokio::task::spawn_blocking(move || drop_sync(&request)).await?
}

fn contents_sync(request: &AgentReleaseContentsRequest) -> Result<AgentReleaseContents> {
    let root = crate::paths::absolute_root(&request.root)?;
    let mut files = Vec::new();
    let mut manifest = None;
    if request.backup {
        let path = crate::paths::real_backup(&root, &request.release)?;
        let decoder = zstd::stream::read::Decoder::new(File::open(path)?)?;
        let mut tar = tar::Archive::new(decoder);
        for entry in tar.entries()? {
            let entry = entry?;
            let relative = safe(&entry.path()?)?;
            if relative.as_os_str() == "." {
                continue;
            }
            let info = file_info(
                &relative,
                entry.header().entry_type(),
                entry.header().mode()?,
                entry.header().size()?,
                entry.link_name()?,
            );
            if info.path == ".shipdeck/release.json" {
                let mut text = String::new();
                entry.take(MAX_READ).read_to_string(&mut text)?;
                manifest = Some(text);
            }
            files.push(info);
            check_entries(files.len())?;
        }
    } else {
        let release = crate::paths::real_release(&root, &request.release)?;
        walk(&release, &release, &mut files)?;
        if let Ok(text) = fs::read_to_string(release.join(".shipdeck/release.json")) {
            manifest = Some(text);
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(AgentReleaseContents { files, manifest })
}

fn read_sync(request: &AgentReleaseReadRequest) -> Result<AgentReleaseRead> {
    let root = crate::paths::absolute_root(&request.root)?;
    let relative = safe(Path::new(&request.path))?;
    let cap = request.cap.clamp(1, MAX_READ);
    let bytes = if request.backup {
        let path = crate::paths::real_backup(&root, &request.release)?;
        let decoder = zstd::stream::read::Decoder::new(File::open(path)?)?;
        let mut tar = tar::Archive::new(decoder);
        let mut found = None;
        for entry in tar.entries()? {
            let entry = entry?;
            let candidate = safe(&entry.path()?)?;
            if candidate == relative {
                let mut bytes = Vec::new();
                entry.take(cap).read_to_end(&mut bytes)?;
                found = Some(bytes);
                break;
            }
        }
        found.with_context(|| format!("{} is not in the backup", request.path))?
    } else {
        let release = crate::paths::real_release(&root, &request.release)?;
        let file = regular_file(&release, &relative)?
            .with_context(|| format!("{} is not a regular file", request.path))?;
        let mut bytes = Vec::new();
        File::open(file)?.take(cap).read_to_end(&mut bytes)?;
        bytes
    };
    Ok(AgentReleaseRead {
        binary: bytes.contains(&0),
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

fn drop_sync(request: &AgentDropRequest) -> Result<AgentOk> {
    let root = crate::paths::absolute_root(&request.root)?;
    if !crate::transfer::safe_id(&request.release) {
        bail!("invalid release name");
    }
    if request.open {
        crate::paths::refuse_current(&root, &request.release)?;
        let releases = crate::paths::real_folder(&root, "releases")?;
        let release = releases.join(&request.release);
        match fs::symlink_metadata(&release) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                fs::remove_dir_all(&release).context("removing the release")?;
            }
            Ok(_) => bail!("the release is not a real directory"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).context("checking the release"),
        }
    }
    if request.backup {
        let backups = crate::paths::real_folder(&root, "backups")?;
        let backup = backups.join(format!("{}.tar.zst", request.release));
        match fs::symlink_metadata(&backup) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                fs::remove_file(&backup).context("removing the backup")?;
            }
            Ok(_) => bail!("the backup is not a regular file"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).context("checking the backup"),
        }
    }
    Ok(AgentOk {})
}

fn walk(root: &Path, folder: &Path, files: &mut Vec<AgentFileInfo>) -> Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .context("release path escaped its folder")?;
        let metadata = fs::symlink_metadata(&path)?;
        let kind = if metadata.file_type().is_symlink() {
            AgentFileKind::Link
        } else if metadata.is_dir() {
            AgentFileKind::Dir
        } else {
            AgentFileKind::File
        };
        let target = if kind == AgentFileKind::Link {
            Some(fs::read_link(&path)?.to_string_lossy().into_owned())
        } else {
            None
        };
        files.push(AgentFileInfo {
            path: slash(relative)?,
            kind,
            mode: mode_string(&metadata),
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            target,
        });
        check_entries(files.len())?;
        if kind == AgentFileKind::Dir {
            walk(root, &path, files)?;
        }
    }
    Ok(())
}

fn file_info(
    path: &Path,
    kind: tar::EntryType,
    mode: u32,
    size: u64,
    target: Option<std::borrow::Cow<'_, Path>>,
) -> AgentFileInfo {
    let kind = if kind.is_symlink() {
        AgentFileKind::Link
    } else if kind.is_dir() {
        AgentFileKind::Dir
    } else {
        AgentFileKind::File
    };
    AgentFileInfo {
        path: path.to_string_lossy().replace('\\', "/"),
        kind,
        mode: format!("{}{:03o}", kind_char(kind), mode & 0o777),
        size: if kind == AgentFileKind::File { size } else { 0 },
        target: if kind == AgentFileKind::Link {
            target.map(|target| target.to_string_lossy().into_owned())
        } else {
            None
        },
    }
}

fn kind_char(kind: AgentFileKind) -> char {
    match kind {
        AgentFileKind::Dir => 'd',
        AgentFileKind::File => '-',
        AgentFileKind::Link => 'l',
    }
}

fn check_entries(count: usize) -> Result<()> {
    if u64::try_from(count)? > MAX_ENTRIES {
        bail!("the release has more than {MAX_ENTRIES} entries");
    }
    Ok(())
}

/// A path relative to the release, with no escape.
fn safe(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => result.push(name),
            Component::CurDir => {}
            _ => bail!("unsafe release path: {}", path.display()),
        }
    }
    Ok(result)
}

fn slash(path: &Path) -> Result<String> {
    path.to_str()
        .map(|text| text.replace('\\', "/"))
        .context("a release path is not UTF-8")
}

fn regular_file(root: &Path, relative: &Path) -> Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(None),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context("checking a release file"),
        }
    }
    match fs::metadata(&path) {
        Ok(metadata) => Ok(metadata.is_file().then_some(path)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context("checking a release file"),
    }
}

#[cfg(unix)]
fn mode_string(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    let kind = if metadata.file_type().is_symlink() {
        'l'
    } else if metadata.is_dir() {
        'd'
    } else {
        '-'
    };
    format!("{kind}{:03o}", mode & 0o777)
}

#[cfg(not(unix))]
fn mode_string(metadata: &fs::Metadata) -> String {
    if metadata.is_dir() { "d755" } else { "-644" }.to_string()
}
