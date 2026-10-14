//! Naming a release from the local source: the app's version, the git commit
//! and whether there are uncommitted changes.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use tokio::process::Command;

use crate::config::Deployable;
use crate::release::ReleaseName;

/// The release name, and where its version came from (for the log).
pub async fn release_name(deployable: &Deployable) -> Result<(ReleaseName, String)> {
    let dir = deployable.project_dir();
    let (version, source) = match deployable.version_file() {
        Some(file) => {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (read_version(&file)?, name)
        }
        None => match latest_tag(&dir).await {
            Some(tag) => (tag, "the latest git tag".to_string()),
            None => (
                "0.0.0".to_string(),
                "nowhere: no version file or git tag, so 0.0.0".to_string(),
            ),
        },
    };
    let (git, dirty) = match git(&dir, &["rev-parse", "--short=7", "HEAD"]).await {
        Some(sha) => {
            let status = git(&dir, &["status", "--porcelain", "--untracked-files=no"]).await;
            (sha, status.is_some_and(|s| !s.is_empty()))
        }
        None => ("nogit".to_string(), false),
    };
    Ok((ReleaseName::new(&version, &git, dirty, Utc::now()), source))
}

/// The version a file names, by its kind: `Cargo.toml`, `package.json`,
/// `pyproject.toml`, or any other file whose first line is the version.
pub fn read_version(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.as_str() {
        "Cargo.toml" => cargo_version(path),
        "package.json" => {
            let text = read(path)?;
            let doc: serde_json::Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            doc.get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("{} has no \"version\"", path.display()))
        }
        "pyproject.toml" => {
            let doc = read_toml(path)?;
            let project = doc.get("project").and_then(|p| p.get("version"));
            let poetry = doc
                .get("tool")
                .and_then(|t| t.get("poetry"))
                .and_then(|p| p.get("version"));
            project
                .or(poetry)
                .and_then(toml::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    anyhow!(
                        "{} has no project.version or tool.poetry.version",
                        path.display()
                    )
                })
        }
        _ => {
            let text = read(path)?;
            let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
                bail!(
                    "{} is empty; its first line should be the version",
                    path.display()
                );
            };
            Ok(line.to_string())
        }
    }
}

/// `git describe --tags --abbrev=0`, without a leading `v`.
async fn latest_tag(dir: &Path) -> Option<String> {
    let tag = git(dir, &["describe", "--tags", "--abbrev=0"]).await?;
    let tag = tag.strip_prefix('v').unwrap_or(&tag);
    (!tag.is_empty()).then(|| tag.to_string())
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
/// workspace's `Cargo.toml`. A workspace root without a package gives its
/// `workspace.package.version`.
fn cargo_version(manifest: &Path) -> Result<String> {
    let doc = read_toml(manifest)?;
    let workspace_version = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(toml::Value::as_str);
    let Some(version) = doc.get("package").and_then(|p| p.get("version")) else {
        return workspace_version
            .map(str::to_string)
            .ok_or_else(|| anyhow!("{} has no package.version", manifest.display()));
    };
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
    if let Some(version) = workspace_version {
        return Ok(version.to_string());
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

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn read_toml(path: &Path) -> Result<toml::Table> {
    read(path)?
        .parse::<toml::Table>()
        .with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
#[path = "version_test.rs"]
mod tests;
