//! Windows: local commands run in the sh.exe of Git for Windows, since
//! Windows has no `sh` of its own.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

/// The sh.exe from `--gitshell-path` or `SHIPDECK_GITSHELL_PATH`.
static CHOSEN: OnceLock<PathBuf> = OnceLock::new();

/// Runs local commands with `path` instead of looking for Git's sh. Set
/// once, before anything runs.
pub fn set_gitshell_path(path: PathBuf) {
    let _ = CHOSEN.set(path);
}

/// The shell for local commands. Its real folder goes first on PATH, as when
/// Git runs a hook, so `cp`, `rm` and the rest are found next to it.
pub(super) async fn sh() -> Result<Command> {
    let path = real(find().await?);
    let folder = path.parent().unwrap_or(&path).to_path_buf();
    let paths = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(folder).chain(std::env::split_paths(&paths));
    let mut command = Command::new(&path);
    command.env(
        "PATH",
        std::env::join_paths(paths).context("putting Git's sh folder on PATH")?,
    );
    Ok(command)
}

/// The first sh.exe found: the one chosen, else the one Git names, else one
/// on PATH, else one where Git for Windows installs.
async fn find() -> Result<PathBuf> {
    if let Some(path) = CHOSEN.get() {
        if path.is_file() {
            return Ok(path.clone());
        }
        bail!(
            "--gitshell-path (or SHIPDECK_GITSHELL_PATH) names {}, which is not a file",
            path.display()
        );
    }
    let mut candidates: Vec<PathBuf> = git_names().await.into_iter().collect();
    let paths = std::env::var_os("PATH").unwrap_or_default();
    candidates.extend(std::env::split_paths(&paths).map(|dir| dir.join("sh.exe")));
    for (var, below) in [
        ("ProgramFiles", r"Git\usr\bin\sh.exe"),
        ("LOCALAPPDATA", r"Programs\Git\usr\bin\sh.exe"),
        ("USERPROFILE", r"scoop\apps\git\current\usr\bin\sh.exe"),
    ] {
        if let Some(base) = std::env::var_os(var) {
            candidates.push(PathBuf::from(base).join(below));
        }
    }
    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Ok(found);
    }
    bail!(
        "shipdeck runs local commands with the sh.exe of Git for Windows and found none \
         (asked `git var GIT_SHELL_PATH`, looked on PATH, in %ProgramFiles%\\Git, \
         %LOCALAPPDATA%\\Programs\\Git and scoop's git); install Git for Windows \
         (https://gitforwindows.org), or set --gitshell-path or SHIPDECK_GITSHELL_PATH \
         to its usr\\bin\\sh.exe"
    )
}

/// The shell Git itself runs scripts with, if Git is on PATH.
async fn git_names() -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["var", "GIT_SHELL_PATH"])
        .stdin(Stdio::null())
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    (out.status.success() && !text.trim().is_empty()).then(|| PathBuf::from(text.trim()))
}

/// Where a link to sh.exe points, so it runs from Git's folder, next to the
/// DLLs and tools it needs; any other path as it is.
fn real(path: PathBuf) -> PathBuf {
    match std::fs::read_link(&path) {
        Ok(target) => path
            .parent()
            .map_or(target.clone(), |dir| dir.join(&target)),
        Err(_) => path,
    }
}
