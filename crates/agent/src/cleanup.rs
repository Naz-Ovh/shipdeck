//! Removal of unfinished files, only after an explicit approval, and of a
//! failed release after its rollback.

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{AgentCleaned, AgentCleanupWork, AgentDiscard, AgentOk};

/// Removes exactly the artifacts the owner approved; nothing else.
pub(crate) async fn cleanup(request: AgentCleanupWork) -> Result<AgentCleaned> {
    tokio::task::spawn_blocking(move || cleanup_sync(&request)).await?
}

/// Removes one failed release, its archive, receipt, reuse lists, and backup.
pub(crate) async fn discard(request: AgentDiscard) -> Result<AgentOk> {
    tokio::task::spawn_blocking(move || discard_sync(&request)).await?
}

fn cleanup_sync(request: &AgentCleanupWork) -> Result<AgentCleaned> {
    let root = crate::paths::absolute_root(&request.root)?;
    let mut removed = Vec::new();
    for artifact in &request.artifacts {
        let relative = approved(artifact)?;
        let path = project_path(&root, &relative)?;
        if !crate::unpack::entry_exists(&path)? {
            continue;
        }
        remove(&path)?;
        removed.push(artifact.clone());
    }
    Ok(AgentCleaned { removed })
}

/// The full path of an approved artifact, through a real project folder.
fn project_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut parts = relative.components();
    match parts.next() {
        Some(Component::Normal(first)) if first == "releases" || first == "backups" => {
            let folder = crate::paths::real_folder(root, &first.to_string_lossy())?;
            let rest = parts.as_path();
            Ok(folder.join(rest))
        }
        _ => Ok(root.join(relative)),
    }
}

fn discard_sync(request: &AgentDiscard) -> Result<AgentOk> {
    let root = crate::paths::absolute_root(&request.root)?;
    if !crate::transfer::safe_id(&request.release) {
        bail!("invalid release name");
    }
    let id = &request.release;
    crate::paths::refuse_current(&root, id)?;
    let releases = crate::paths::real_folder(&root, "releases")?;
    remove_if_there(&releases.join(id))?;
    let backups = crate::paths::real_folder(&root, "backups")?;
    remove_if_there(&backups.join(format!("{id}.tar.zst")))?;
    for folder in [&releases, &backups] {
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.starts_with(&format!(".{id}.")) {
                    remove(&entry.path())?;
                }
            }
        }
    }
    Ok(AgentOk {})
}

/// Checks an artifact path from `work/get` before deleting anything.
fn approved(artifact: &str) -> Result<PathBuf> {
    let relative = Path::new(artifact);
    if artifact.is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("refusing an unfinished path: {artifact:?}");
    }
    let parts: Vec<&str> = relative
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    let name = parts
        .last()
        .copied()
        .context("unfinished path has no name")?;
    let inside = matches!(
        parts.as_slice(),
        [folder, _] if *folder == "releases" || *folder == "backups"
    );
    let switch =
        parts.len() == 1 && name.starts_with(".current-switch-") && name.ends_with(".partial");
    let known = name.starts_with('.')
        && (crate::files::ends_with(name, ".partial")
            || crate::files::ends_with(name, ".part")
            || crate::files::ends_with(name, ".paths")
            || crate::files::ends_with(name, ".checked")
            || crate::files::ends_with(name, ".sha256")
            || crate::files::ends_with(name, ".tar.zst")
            || crate::files::ends_with(name, ".files")
            || name.contains(".reuse."));
    if !(switch || (inside && known)) {
        bail!("refusing an unfinished path: {artifact:?}");
    }
    Ok(relative.to_path_buf())
}

fn remove_if_there(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => remove(path),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).context("checking a release path"),
    }
}

/// Removes a file, a link, or a directory tree; never follows a link.
fn remove(path: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("checking {}", path.display()))?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    } else {
        bail!("{} is neither a file nor a folder", path.display());
    }
    Ok(())
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
