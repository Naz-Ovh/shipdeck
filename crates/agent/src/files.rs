//! Native file operations on a deployment's current release.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use shipdeck_protocol::message::agent::{
    AgentLinkShared, AgentOk, HashCurrent, HashCurrentResult, InspectWork, WorkInspection,
};
use tokio::io::AsyncReadExt;

/// Reads the live target and unfinished artifacts without changing them.
pub(crate) async fn inspect_work(request: InspectWork) -> Result<WorkInspection> {
    let root = Path::new(&request.root);
    if !root.is_absolute() {
        bail!("deployment root must be absolute");
    }
    let current = crate::paths::current_release(root)?;
    let mut unfinished = Vec::new();
    match tokio::fs::symlink_metadata(root.join("releases")).await {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => bail!("releases/ must be a real directory"),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("checking the releases directory"),
    }
    match tokio::fs::read_dir(root.join("releases")).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await? {
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                if name.starts_with('.')
                    && (name.ends_with(".partial")
                        || name.ends_with(".tar.zst")
                        || name.ends_with(".sha256")
                        || Path::new(name)
                            .extension()
                            .is_some_and(|ext| ext == "paths")
                        || name.ends_with(".checked")
                        || name.contains(".reuse."))
                {
                    unfinished.push(format!("releases/{name}"));
                }
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("reading release artifacts"),
    }
    match tokio::fs::symlink_metadata(root.join("backups")).await {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => bail!("backups/ must be a real directory"),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("checking the backups directory"),
    }
    match tokio::fs::read_dir(root.join("backups")).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await? {
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                if name.starts_with('.')
                    && (ends_with(name, ".part")
                        || ends_with(name, ".files")
                        || ends_with(name, ".tar.zst.part"))
                {
                    unfinished.push(format!("backups/{name}"));
                }
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("reading backup artifacts"),
    }
    match tokio::fs::read_dir(root).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await? {
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                if name.starts_with(".current-switch-") && name.ends_with(".partial") {
                    unfinished.push(name.to_string());
                }
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("reading the project directory"),
    }
    unfinished.sort();
    let uncertain = match &current {
        Some(release) => crate::state::unconfirmed_activation(&request.root, release)
            .await?
            .map(|_| release.clone()),
        None => None,
    };
    Ok(WorkInspection {
        current,
        unfinished,
        uncertain,
    })
}

/// Links each shared folder into a prepared release, creating it once.
pub(crate) async fn link_shared(request: AgentLinkShared) -> Result<AgentOk> {
    if request.shared.len() > 1000 {
        bail!("too many shared folders");
    }
    let root = Path::new(&request.root);
    if !root.is_absolute() || !crate::transfer::safe_id(&request.release) {
        bail!("invalid deployment root or release name");
    }
    let releases = crate::paths::real_folder(root, "releases")?;
    let release = crate::paths::real_release(root, &request.release)?;
    if release.parent() != Some(releases.as_path()) {
        bail!("the prepared release is outside releases/");
    }
    let shared_root = crate::paths::real_folder(root, "shared")?;
    for entry in &request.shared {
        let relative = crate::unpack::safe_relative(Path::new(entry))?;
        let shared = ensure_dirs(&shared_root, relative.as_path()).await?;
        let link = release.join(&relative);
        let parent = link.parent().context("shared link has no parent")?;
        ensure_existing_dirs(&release, parent).await?;
        match tokio::fs::symlink_metadata(&link).await {
            Ok(_) => bail!(
                "{} came in the release, but it is shared",
                relative.display()
            ),
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err).context("checking the shared link"),
        }
        create_link(&shared, &link).await?;
    }
    Ok(AgentOk {})
}

/// Creates `base/relative` below `root`, refusing symbolic links on the way.
async fn ensure_dirs(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            bail!("unsafe shared path");
        };
        path.push(name);
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!("{} is not a directory", path.display()),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                tokio::fs::create_dir(&path)
                    .await
                    .with_context(|| format!("creating {}", path.display()))?;
                crate::owner::apply(&path)?;
            }
            Err(err) => return Err(err).context("checking a shared folder"),
        }
    }
    Ok(path)
}

/// Checks that every existing parent of `link` under `release` is a real
/// directory, so a link cannot be written through a symbolic link.
async fn ensure_existing_dirs(release: &Path, parent: &Path) -> Result<()> {
    let relative = parent
        .strip_prefix(release)
        .context("shared link escapes the release")?;
    let mut path = release.to_path_buf();
    for part in relative.components() {
        path.push(part.as_os_str());
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!("{} is not a directory", path.display()),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                tokio::fs::create_dir(&path).await?;
                crate::owner::apply(&path)?;
            }
            Err(err) => return Err(err).context("checking the release folder"),
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn create_link(target: &Path, link: &Path) -> Result<()> {
    tokio::fs::symlink(target, link)
        .await
        .with_context(|| format!("linking {}", link.display()))
}

#[cfg(not(unix))]
async fn create_link(_target: &Path, _link: &Path) -> Result<()> {
    bail!("shared folders are linked on Linux servers only")
}

/// Hashes regular files without following links inside the release.
pub(crate) async fn hash_current(request: &HashCurrent) -> Result<HashCurrentResult> {
    let root = Path::new(&request.root);
    if !root.is_absolute() {
        bail!("deployment root must be absolute");
    }
    let current = tokio::fs::canonicalize(root.join("current"))
        .await
        .context("opening the current release")?;
    let mut hashes = BTreeMap::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    for path in &request.paths {
        let Some(file) = safe_file(&current, path).await? else {
            continue;
        };
        let mut file = tokio::fs::File::open(&file)
            .await
            .with_context(|| format!("opening {}", file.display()))?;
        let mut hash = Sha256::new();
        loop {
            let count = file
                .read(&mut buffer)
                .await
                .context("reading a release file")?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        hashes.insert(
            path.clone(),
            base16ct::lower::encode_string(&hash.finalize()),
        );
    }
    Ok(HashCurrentResult { hashes })
}

/// Whether one of our own artifact names ends in `suffix`.
pub(crate) fn ends_with(name: &str, suffix: &str) -> bool {
    name.ends_with(suffix)
}

/// Rejects unsafe paths and skips missing files and symbolic links.
async fn safe_file(current: &Path, relative: &str) -> Result<Option<PathBuf>> {
    let path = Path::new(relative);
    if relative.is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("unsafe release path {relative:?}");
    }
    let mut full = current.to_path_buf();
    for part in path.components() {
        full.push(part.as_os_str());
        match tokio::fs::symlink_metadata(&full).await {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(None),
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("checking {}", full.display())),
        }
    }
    let metadata = tokio::fs::metadata(&full).await?;
    Ok(metadata.is_file().then_some(full))
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
