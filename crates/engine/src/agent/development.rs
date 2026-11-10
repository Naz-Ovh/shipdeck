//! Explicit local-agent installation for development. The upload starts in a
//! private user folder, but root copies and checks it before anything runs.

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context, Result, bail};
use object::{Architecture, BinaryFormat, Object};
use sha2::{Digest, Sha256};

use super::install;
use crate::ssh::{Session, sh_quote};

/// Selects a local Linux agent binary for development, never a release fallback.
pub(super) const BINARY_ENV: &str = "SHIPDECK_AGENT_BINARY";

/// Uploads a checked Linux executable, then installs the root-owned copy.
pub(super) async fn install(
    session: &Session,
    prefix: &str,
    local: &OsStr,
    target: &str,
    label: &str,
    status: &(dyn Fn(&str) + Sync),
    bytes: &(dyn Fn(u64, u64) + Sync),
) -> Result<String> {
    if local.is_empty() {
        bail!("{BINARY_ENV} must name a local Linux agent binary");
    }
    let local = Path::new(local);
    status("reading the local agent binary");
    let data = tokio::fs::read(local)
        .await
        .with_context(|| format!("reading development agent {}", local.display()))?;
    check_target(&data, target)?;
    let digest = base16ct::lower::encode_string(&Sha256::digest(&data));
    status("creating a private folder for the upload");
    let folder = private_folder(session).await?;
    let partial = format!("{folder}/agent.partial");
    status("uploading the local agent");
    let uploaded = session
        .upload_with_cat(&data, &partial, bytes)
        .await
        .with_context(|| format!("uploading development agent; unfinished file: {partial}"));
    if let Err(err) = uploaded {
        remove_folder(session, &folder).await;
        return Err(err);
    }
    status("installing the uploaded agent");
    let result = install::install_uploaded(session, prefix, &partial, &digest, label).await;
    remove_folder(session, &folder).await;
    result
}

/// Removes a user-owned upload folder; a failure is reported, not hidden.
async fn remove_folder(session: &Session, folder: &str) {
    let script = format!("rm -rf -- {}", sh_quote(folder));
    match session.exec_raw(&script).await {
        Ok(output) if output.success() => {}
        Ok(output) => eprintln!(
            "shipdeck: {}: could not remove development upload folder {folder}: {}",
            session.label,
            output.stderr.trim()
        ),
        Err(err) => eprintln!(
            "shipdeck: {}: could not remove development upload folder {folder}: {err:#}",
            session.label
        ),
    }
}

/// A fresh folder under tmp, made by `mktemp` and checked before use.
async fn private_folder(session: &Session) -> Result<String> {
    let output = session
        .exec_raw(&temp_dir_script())
        .await
        .context("creating a private temporary folder for the agent")?
        .check("creating a private temporary folder for the agent")?;
    let folder = output.stdout.trim();
    if !folder.starts_with('/') {
        bail!("the server did not create a private temporary folder");
    }
    Ok(folder.to_string())
}

/// `mktemp -d`, then ownership, mode, and symlink checks.
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

/// Checks format and CPU before sending a host binary to another platform.
fn check_target(data: &[u8], target: &str) -> Result<()> {
    let file = object::File::parse(data).context("development agent is not an executable")?;
    if file.format() != BinaryFormat::Elf {
        bail!(
            "development agent must be a Linux ELF executable, not {:?}",
            file.format()
        );
    }
    let matches = matches!(
        (file.architecture(), target.split('-').next()),
        (Architecture::X86_64, Some("x86_64")) | (Architecture::Aarch64, Some("aarch64"))
    );
    if !matches {
        bail!(
            "development agent CPU {:?} does not match server target {target}",
            file.architecture()
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "development_tests.rs"]
mod tests;
