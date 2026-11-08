//! Explicit local-agent installation; published downloads never enter this path.

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context, Result, bail};
use object::{Architecture, BinaryFormat, Object};
use sha2::{Digest, Sha256};

use super::install;
use crate::ssh::{Session, sh_quote};

/// Selects a local Linux agent binary for development, never a release fallback.
pub(super) const BINARY_ENV: &str = "SHIPDECK_AGENT_BINARY";

/// Uploads a checked Linux executable into a fresh private remote directory.
pub(super) async fn install(
    session: &Session,
    local: &OsStr,
    target: &str,
    version: &str,
) -> Result<String> {
    if local.is_empty() {
        bail!("{BINARY_ENV} must name a local Linux agent binary");
    }
    let local = Path::new(local);
    let data = tokio::fs::read(local)
        .await
        .with_context(|| format!("reading development agent {}", local.display()))?;
    check_target(&data, target)?;
    let digest = base16ct::lower::encode_string(&Sha256::digest(&data));
    let folder = install::temp_folder(session).await?;
    let partial = format!("{folder}/agent.partial");
    let path = format!("{folder}/shipdeck-agent-{version}");
    session
        .upload_with_cat(&data, &partial, |_, _| {})
        .await
        .with_context(|| format!("uploading development agent; unfinished file: {partial}"))?;
    let script = finalize_script(&partial, &path, &digest);
    session
        .exec_raw(&script)
        .await?
        .check("verifying the development agent")?;
    install::probe(session, &path, version)
        .await
        .context("development agent must run on the server and match the engine version")?;
    Ok(path)
}

/// Checks format and CPU before sending a host binary to a different platform.
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

/// Checks exact uploaded bytes before making the private executable runnable.
fn finalize_script(partial: &str, path: &str, digest: &str) -> String {
    format!(
        "set -e\nactual=$(sha256sum {partial})\nactual=${{actual%% *}}\n\
         [ \"$actual\" = {digest} ] || {{ echo 'development agent checksum mismatch' >&2; exit 1; }}\n\
         chmod 0700 {partial}\nmv {partial} {path}",
        partial = sh_quote(partial),
        path = sh_quote(path),
        digest = sh_quote(digest),
    )
}

#[cfg(test)]
#[path = "development_tests.rs"]
mod tests;
