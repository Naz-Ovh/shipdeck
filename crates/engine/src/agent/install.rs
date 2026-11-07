//! Putting the agent on the server: the home folder, the tmp fallback, and
//! the probes. The server downloads it; nothing is uploaded through SSH.

use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::binary;
use crate::ssh::{Output, Session, sh_quote};

/// A 4 MiB archive on a slow link, with a hard stop.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45);

/// `$HOME`, as an absolute path.
pub(super) async fn home(session: &Session) -> Result<String> {
    let home = session
        .exec_raw("printf '%s' \"$HOME\"")
        .await
        .context("asking the server for the home folder")?
        .stdout;
    let home = home.trim();
    if !home.starts_with('/') {
        bail!("the server did not say where the home folder is");
    }
    Ok(home.to_string())
}

/// Makes sure the agent in `folder` exists and runs; returns the path it
/// started from. A home folder that is `noexec` or not writable falls back to
/// a fresh private folder in tmp.
pub(super) async fn install(
    session: &Session,
    folder: &str,
    target: &str,
    version: &str,
) -> Result<String> {
    let path = format!("{folder}/shipdeck-agent-{version}");
    if probe(session, &path, version).await.is_ok() {
        return Ok(path);
    }
    let home_error = match place(session, &path, target, version).await {
        Ok(()) => {
            remove_older(session, folder, version).await;
            return Ok(path);
        }
        Err(err) => format!("{err:#}"),
    };

    let folder = temp_folder(session).await?;
    let path = format!("{folder}/shipdeck-agent-{version}");
    place(session, &path, target, version)
        .await
        .with_context(|| {
            format!("the agent cannot run in the home folder ({home_error}) or in {folder}")
        })?;
    Ok(path)
}

/// Downloads and runs the agent at `path`.
async fn place(session: &Session, path: &str, target: &str, version: &str) -> Result<()> {
    let script = binary::install_script(target, path);
    session
        .exec_raw_within(&script, INSTALL_TIMEOUT)
        .await?
        .check("downloading the agent")?;
    probe(session, path, version).await
}

/// A fresh folder under tmp, made by `mktemp` and checked before use. Its
/// name is unpredictable, so another user cannot pre-create or race it.
async fn temp_folder(session: &Session) -> Result<String> {
    let output = session
        .exec_raw(&temp_dir_script())
        .await
        .context("creating a private temporary folder for the agent")?;
    let output = output.check("creating a private temporary folder for the agent")?;
    let folder = output.stdout.trim();
    if !folder.starts_with('/') {
        bail!("the server did not create a private temporary folder");
    }
    Ok(folder.to_string())
}

/// `mktemp -d`, then the ownership, mode and symlink checks.
fn temp_dir_script() -> String {
    format!(
        "set -e\n\
         command -v mktemp >/dev/null 2>&1 || {{ echo 'mktemp is not installed' >&2; exit 1; }}\n\
         dir=$(mktemp -d \"${{TMPDIR:-/tmp}}/.shipdeck.XXXXXXXXXX\")\n\
         {checks}\
         printf '%s' \"$dir\"\n",
        checks = private_checks("\"$dir\""),
    )
}

/// The checks that `folder` is ours, private, and not a symlink.
fn private_checks(folder: &str) -> String {
    format!(
        "[ ! -L {folder} ] || {{ echo 'the agent folder is a symlink' >&2; exit 1; }}\n\
         [ \"$(stat -c %u {folder})\" = \"$(id -u)\" ] || {{ echo 'the agent folder belongs to another user' >&2; exit 1; }}\n\
         [ \"$(stat -c %a {folder})\" = \"700\" ] || {{ echo 'the agent folder is not private' >&2; exit 1; }}\n"
    )
}

/// Runs `<path> --version` and checks the version it prints.
async fn probe(session: &Session, path: &str, version: &str) -> Result<()> {
    let output = session
        .exec_raw(&format!("{} --version 2>/dev/null", sh_quote(path)))
        .await?;
    check_probe(&output, version).with_context(|| format!("running the agent at {path}"))
}

fn check_probe(output: &Output, version: &str) -> Result<()> {
    if !output.success() {
        bail!("it exited with code {}", output.code);
    }
    if !output.stdout.contains(version) {
        bail!("it printed {:?}", output.stdout.trim());
    }
    Ok(())
}

/// Removes older versions in `folder`; a failure is reported, not fatal.
async fn remove_older(session: &Session, folder: &str, version: &str) {
    let keep = format!("shipdeck-agent-{version}");
    let script = format!(
        "find {} -maxdepth 1 -type f -name 'shipdeck-agent-*' ! -name {} -delete",
        sh_quote(folder),
        sh_quote(&keep)
    );
    match session.exec_raw(&script).await {
        Ok(output) if !output.success() => eprintln!(
            "shipdeck: {}: could not remove older agents: {}",
            session.label,
            output.stderr.trim()
        ),
        Ok(_) => {}
        Err(err) => eprintln!(
            "shipdeck: {}: could not remove older agents: {err:#}",
            session.label
        ),
    }
}

#[cfg(test)]
#[path = "install_test.rs"]
mod tests;
