//! The releases and backups in a deployment folder, read natively.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{
    AgentRelease, AgentReleaseFile, AgentReleaseFiles, AgentReleaseFilesRequest, AgentReleaseKind,
    AgentReleases,
};

/// How large a `release.json` may be before it is ignored.
const MAX_MANIFEST: u64 = 1024 * 1024;
/// How many entries one release may hold.
const MAX_ENTRIES: u64 = 1_000_000;

/// Lists the open releases and backups outside the asynchronous SSH reader.
pub(crate) async fn list(root: String) -> Result<AgentReleases> {
    tokio::task::spawn_blocking(move || list_sync(Path::new(&root))).await?
}

/// Lists one release's paths outside the asynchronous SSH reader.
pub(crate) async fn files(request: AgentReleaseFilesRequest) -> Result<AgentReleaseFiles> {
    tokio::task::spawn_blocking(move || files_sync(&request)).await?
}

fn list_sync(root: &Path) -> Result<AgentReleases> {
    if !root.is_absolute() {
        bail!("deployment root must be absolute");
    }
    let current = crate::paths::current_release(root)?;
    let releases = crate::paths::real_folder_if_there(root, "releases")?;
    let backups = crate::paths::real_folder_if_there(root, "backups")?;
    let mut entries: Vec<AgentRelease> = Vec::new();
    match releases.as_deref().map(std::fs::read_dir) {
        Some(Ok(found)) => {
            for entry in found {
                let entry = entry?;
                let name = entry.file_name();
                let Some(id) = name.to_str() else { continue };
                if !crate::transfer::safe_id(id) || !is_real_dir(&entry)? {
                    continue;
                }
                entries.push(AgentRelease {
                    id: id.to_string(),
                    kind: AgentReleaseKind::Open,
                    current: current.as_deref() == Some(id),
                    size_bytes: Some(size_of(&entry.path(), 0)?),
                    backup: false,
                    manifest: manifest_of(&entry.path())?,
                });
            }
        }
        Some(Err(err)) => return Err(err).context("reading the releases directory"),
        None => {}
    }
    match backups.as_deref().map(std::fs::read_dir) {
        Some(Ok(found)) => {
            for entry in found {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                let Some(id) = name.strip_suffix(".tar.zst") else {
                    continue;
                };
                if !crate::transfer::safe_id(id) || !entry.file_type()?.is_file() {
                    continue;
                }
                let size = entry.metadata().ok().map(|meta| meta.len());
                match entries.iter_mut().find(|release| release.id == id) {
                    Some(release) => release.backup = true,
                    None => entries.push(AgentRelease {
                        id: id.to_string(),
                        kind: AgentReleaseKind::Compressed,
                        current: false,
                        size_bytes: size,
                        backup: true,
                        manifest: None,
                    }),
                }
            }
        }
        Some(Err(err)) => return Err(err).context("reading the backups directory"),
        None => {}
    }
    Ok(AgentReleases { current, entries })
}

fn files_sync(request: &AgentReleaseFilesRequest) -> Result<AgentReleaseFiles> {
    let root = crate::paths::absolute_root(&request.root)?;
    let release = crate::paths::real_release(&root, &request.release)?;
    let mut files = Vec::new();
    walk(&release, &release, &mut files)?;
    Ok(AgentReleaseFiles { files })
}

fn walk(root: &Path, folder: &Path, files: &mut Vec<AgentReleaseFile>) -> Result<()> {
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .context("release path escaped its folder")?;
        let Some(name) = relative.to_str() else {
            bail!("a release path is not UTF-8");
        };
        let kind = entry.file_type()?;
        files.push(AgentReleaseFile {
            path: name.replace('\\', "/"),
            is_dir: kind.is_dir(),
        });
        if u64::try_from(files.len())? > MAX_ENTRIES {
            bail!("the release has more than {MAX_ENTRIES} entries");
        }
        if kind.is_dir() {
            walk(root, &path, files)?;
        }
    }
    Ok(())
}

fn is_real_dir(entry: &std::fs::DirEntry) -> Result<bool> {
    let metadata = entry.metadata()?;
    Ok(metadata.is_dir() && !entry.file_type()?.is_symlink())
}

fn size_of(path: &Path, depth: u32) -> Result<u64> {
    if depth > 128 {
        bail!("release folders are nested too deeply");
    }
    let mut total = 0_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            total = total
                .checked_add(size_of(&entry.path(), depth + 1)?)
                .context("release size overflow")?;
        } else if kind.is_file() {
            total = total
                .checked_add(entry.metadata()?.len())
                .context("release size overflow")?;
        }
    }
    Ok(total)
}

fn manifest_of(release: &Path) -> Result<Option<String>> {
    let path: PathBuf = release.join(".shipdeck/release.json");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            if metadata.len() > MAX_MANIFEST {
                return Ok(None);
            }
            Ok(Some(
                std::fs::read_to_string(path).context("reading release.json")?,
            ))
        }
        Ok(_) => Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context("checking release.json"),
    }
}

#[cfg(test)]
#[path = "releases_tests.rs"]
mod tests;
