//! Where the agent keeps its files on the server: `/var/lib/shipdeck`, owned by
//! root and written only by the agent. Why there: docs/feature/agent.md,
//! "Where the agent lives".

use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

/// The agent's folder: its program, its log and every user's events.
pub(crate) const SHIPDECK_ROOT: &str = "/var/lib/shipdeck";

/// One SSH user's event database, so users never see each other's history.
///
/// # Errors
///
/// Fails on a name that would leave the users folder.
pub(crate) fn state_file(root: &Path, operator: &str) -> Result<PathBuf> {
    if operator.is_empty() || operator == "." || operator == ".." || operator.contains('/') {
        bail!("invalid user name for the state folder: {operator:?}");
    }
    Ok(root.join("users").join(operator).join("state.db"))
}

/// Creates `path` readable by root only when missing, and refuses anything but
/// a real folder, so a link can never redirect what root writes.
///
/// # Errors
///
/// Fails when the folder cannot be made or is a link or a file.
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(err).with_context(|| format!("creating {}", path.display()));
        }
    }
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("{} must be a folder, not a link or a file", path.display());
    }
    Ok(())
}

/// Adds one error line to `/var/lib/shipdeck/agent.log`.
///
/// # Errors
///
/// Fails when the log cannot be opened or written.
pub fn record_error(message: &str) -> Result<()> {
    append_line(&Path::new(SHIPDECK_ROOT).join("agent.log"), message)
}

/// Appends `<unix seconds> <pid> <message>` to `log`, readable by root only.
fn append_line(log: &Path, message: &str) -> Result<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(log)
        && !metadata.is_file()
    {
        bail!("{} must be a file, not a link", log.display());
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(log)
        .with_context(|| format!("opening {}", log.display()))?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    writeln!(file, "{seconds} {} {}", std::process::id(), message.trim())
        .with_context(|| format!("writing {}", log.display()))
}

#[cfg(test)]
#[path = "place_tests.rs"]
mod tests;
