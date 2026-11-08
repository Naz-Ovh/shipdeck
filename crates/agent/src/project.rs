//! Native preparation and atomic live-release switching.

use std::path::Path;

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{AgentEventAppend, AgentOk, InspectWork, SwitchRelease};

/// Creates the standard directories in an existing deployment root.
pub(crate) async fn prepare(request: InspectWork) -> Result<AgentOk> {
    let root = Path::new(&request.root);
    if !root.is_absolute() || !tokio::fs::metadata(root).await?.is_dir() {
        bail!("deployment root must be an existing absolute directory");
    }
    for name in ["releases", "backups", "shared"] {
        let path = root.join(name);
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => bail!("{name}/ must be a real directory"),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir(&path)
                    .await
                    .with_context(|| format!("creating {name}"))?;
            }
            Err(err) => return Err(err).with_context(|| format!("checking {name}")),
        }
        crate::owner::apply(&path)?;
    }
    Ok(AgentOk {})
}

/// Switches `current` in one rename after confirming the release exists.
pub(crate) async fn switch(request: SwitchRelease) -> Result<AgentOk> {
    validate_release(&request).await?;
    record(&request, "switch_started").await?;
    switch_link(&request).await?;
    record(&request, "switch_completed").await?;
    Ok(AgentOk {})
}

async fn validate_release(request: &SwitchRelease) -> Result<()> {
    let root = Path::new(&request.root);
    if !root.is_absolute() || !crate::transfer::safe_id(&request.release) {
        bail!("invalid deployment root or release name");
    }
    let release = root.join("releases").join(&request.release);
    let metadata = tokio::fs::symlink_metadata(&release)
        .await
        .context("checking the release")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("the target release is not a directory");
    }
    Ok(())
}

async fn switch_link(request: &SwitchRelease) -> Result<()> {
    let root = Path::new(&request.root);
    let temporary = root.join(format!(
        ".current-switch-{}.partial",
        crate::transfer::random_token().await?
    ));
    let target = Path::new("releases").join(&request.release);
    create_link(&target, &temporary).await?;
    tokio::fs::rename(&temporary, root.join("current"))
        .await
        .context("switching the live release")?;
    Ok(())
}

#[cfg(unix)]
async fn create_link(target: &Path, link: &Path) -> Result<()> {
    tokio::fs::symlink(target, link)
        .await
        .context("creating the live release link")
}

#[cfg(not(unix))]
async fn create_link(_target: &Path, _link: &Path) -> Result<()> {
    bail!("switching releases is supported on Linux servers only")
}

async fn record(request: &SwitchRelease, kind: &str) -> Result<()> {
    crate::state::append(AgentEventAppend {
        project: request.root.clone(),
        operation: request.release.clone(),
        kind: kind.to_string(),
        detail: String::new(),
    })
    .await
    .context("saving the switch event")?;
    Ok(())
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
