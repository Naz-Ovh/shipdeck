//! Path checks shared by the agent's file operations: every project folder
//! used for a change is a real directory under the project, never a link.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// An absolute project folder.
pub(crate) fn absolute_root(root: &str) -> Result<PathBuf> {
    let path = Path::new(root);
    if !path.is_absolute() {
        bail!("deployment root must be absolute");
    }
    Ok(path.to_path_buf())
}

/// `root/name` when it is a real directory directly inside `root`.
pub(crate) fn real_folder(root: &Path, name: &str) -> Result<PathBuf> {
    let canonical_root = fs::canonicalize(root).context("opening the project folder")?;
    let path = root.join(name);
    let metadata =
        fs::symlink_metadata(&path).with_context(|| format!("checking {}", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("{} must be a real directory", path.display());
    }
    let canonical = fs::canonicalize(&path).context("opening the project folder")?;
    if canonical.parent() != Some(canonical_root.as_path()) {
        bail!("{} must sit directly inside the project", path.display());
    }
    Ok(canonical)
}

/// `root/name` when it is a real directory, or `None` when it is absent.
pub(crate) fn real_folder_if_there(root: &Path, name: &str) -> Result<Option<PathBuf>> {
    match fs::symlink_metadata(root.join(name)) {
        Ok(_) => real_folder(root, name).map(Some),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("checking {name}/")),
    }
}

/// `releases/<release>` when it is a real directory.
pub(crate) fn real_release(root: &Path, release: &str) -> Result<PathBuf> {
    if !crate::transfer::safe_id(release) {
        bail!("invalid release name");
    }
    let releases = real_folder(root, "releases")?;
    let path = releases.join(release);
    let metadata = fs::symlink_metadata(&path).context("checking the release")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("the release is not a real directory");
    }
    if fs::canonicalize(&path)?.parent() != Some(releases.as_path()) {
        bail!("the release is outside releases/");
    }
    Ok(path)
}

/// `backups/<release>.tar.zst` when it is a real file.
pub(crate) fn real_backup(root: &Path, release: &str) -> Result<PathBuf> {
    if !crate::transfer::safe_id(release) {
        bail!("invalid release name");
    }
    let backups = real_folder(root, "backups")?;
    let path = backups.join(format!("{release}.tar.zst"));
    let metadata = fs::symlink_metadata(&path).context("checking the backup")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("the backup is not a regular file");
    }
    Ok(path)
}

/// The release `current` points to, by name.
pub(crate) fn current_release(root: &Path) -> Result<Option<String>> {
    match fs::read_link(root.join("current")) {
        Ok(path) => Ok(Some(
            path.file_name()
                .and_then(|name| name.to_str())
                .context("the current release name is not UTF-8")?
                .to_string(),
        )),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context("reading the current release"),
    }
}

/// Fails when `release` is the one `current` points at.
pub(crate) fn refuse_current(root: &Path, release: &str) -> Result<()> {
    if current_release(root)?.as_deref() == Some(release) {
        bail!("{release} is the live release and is never removed");
    }
    Ok(())
}
