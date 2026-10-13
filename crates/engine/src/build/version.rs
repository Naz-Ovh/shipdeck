//! Naming a release from the local source: the Cargo version, the git commit
//! and whether there are uncommitted changes.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use tokio::process::Command;

use crate::config::Deployable;
use crate::release::ReleaseName;

pub async fn release_name(deployable: &Deployable) -> Result<ReleaseName> {
    let version = cargo_version(&deployable.version_file())?;
    let dir = deployable.build_dir();
    let (git, dirty) = match git(&dir, &["rev-parse", "--short=7", "HEAD"]).await {
        Some(sha) => {
            let status = git(&dir, &["status", "--porcelain", "--untracked-files=no"]).await;
            (sha, status.is_some_and(|s| !s.is_empty()))
        }
        None => ("nogit".to_string(), false),
    };
    Ok(ReleaseName::new(&version, &git, dirty, Utc::now()))
}

/// Who is deploying: git's `user.name`, else the login name.
pub async fn deployer(dir: &Path) -> String {
    match git(dir, &["config", "user.name"]).await {
        Some(name) if !name.is_empty() => name,
        _ => std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
    }
}

async fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `package.version`, following `version.workspace = true` up to the
/// workspace's `Cargo.toml`.
fn cargo_version(manifest: &Path) -> Result<String> {
    let doc = read_toml(manifest)?;
    let version = doc
        .get("package")
        .and_then(|p| p.get("version"))
        .ok_or_else(|| anyhow!("{} has no package.version", manifest.display()))?;
    if let Some(version) = version.as_str() {
        return Ok(version.to_string());
    }

    let inherited = version
        .get("workspace")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    if !inherited {
        return Err(anyhow!(
            "{} has an unsupported package.version",
            manifest.display()
        ));
    }
    let mut dir = manifest.parent();
    while let Some(current) = dir {
        let candidate = current.join("Cargo.toml");
        if candidate != manifest
            && candidate.is_file()
            && let Some(version) = read_toml(&candidate)?
                .get("workspace")
                .and_then(|w| w.get("package"))
                .and_then(|p| p.get("version"))
                .and_then(toml::Value::as_str)
        {
            return Ok(version.to_string());
        }
        dir = current.parent();
    }
    Err(anyhow!(
        "{} inherits its version, but no workspace Cargo.toml above it sets workspace.package.version",
        manifest.display()
    ))
}

fn read_toml(path: &Path) -> Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.parse::<toml::Table>()
        .with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
#[path = "version_test.rs"]
mod tests;
