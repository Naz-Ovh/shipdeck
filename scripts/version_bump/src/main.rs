//! Update release versions without application dependencies.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// One validated file update and its original contents for recovery.
struct FileChange {
    path: PathBuf,
    original: String,
    updated: String,
}

impl FileChange {
    /// Read the original file before any release files are written.
    fn read(path: PathBuf) -> Result<Self> {
        let original = fs::read_to_string(&path)?;
        Ok(Self {
            path,
            original,
            updated: String::new(),
        })
    }
}

/// Parse a stable version without limiting the size of its numbers.
fn version(value: &str) -> Result<Vec<&str>> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
        })
    {
        return Err("Expected a version like 0.6.3.".into());
    }
    Ok(parts)
}

/// Compare version numbers by digit count and then by their digits.
fn newer(new: &str, old: &str) -> Result<bool> {
    let new = version(new)?;
    let old = version(old)?;
    Ok(new
        .iter()
        .map(|part| (part.len(), *part))
        .cmp(old.iter().map(|part| (part.len(), *part)))
        .is_gt())
}

/// Find the files with release URLs that need updating.
fn versioned_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = vec![
        root.join("README.md"),
        root.join("docs/visual/prototype/desktop.html"),
    ];
    for entry in fs::read_dir(root.join("examples"))? {
        let path = entry?.path().join("deploy.yaml");
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Update only the local workspace packages in Cargo's generated lockfile.
fn lockfile(content: &str, old: &str, new: &str) -> Result<String> {
    let packages = [
        "shipdeck",
        "shipdeck-agent",
        "shipdeck-engine",
        "shipdeck-protocol",
        "shipdeck-tui",
    ];
    let mut found = Vec::with_capacity(packages.len());
    let mut updated = String::with_capacity(content.len());
    for section in content.split("[[package]]") {
        if !updated.is_empty() {
            updated.push_str("[[package]]");
        }
        let name = section.lines().find_map(|line| {
            line.strip_prefix("name = \"")
                .and_then(|value| value.strip_suffix('"'))
        });
        if let Some(name) = name.filter(|name| packages.contains(name)) {
            if found.contains(&name)
                || section.lines().any(|line| line.starts_with("source = "))
                || !section
                    .lines()
                    .any(|line| line == format!("version = \"{old}\""))
            {
                return Err(format!("Unexpected lockfile entry for {name}.").into());
            }
            found.push(name);
            updated.push_str(&section.replacen(
                &format!("version = \"{old}\""),
                &format!("version = \"{new}\""),
                1,
            ));
        } else {
            updated.push_str(section);
        }
    }
    if found.len() != packages.len() {
        return Err("Cargo.lock is missing a workspace package.".into());
    }
    Ok(updated)
}

/// Apply prepared changes, reporting every failure to restore an original.
fn apply(changes: &[FileChange]) -> Result<()> {
    for (index, change) in changes.iter().enumerate() {
        if let Err(error) = fs::write(&change.path, &change.updated) {
            let mut message = format!("Could not write {}: {error}", change.path.display());
            for original in changes.iter().take(index + 1) {
                if let Err(restore) = fs::write(&original.path, &original.original) {
                    use std::fmt::Write;
                    write!(
                        message,
                        "\nCould not restore {}: {restore}",
                        original.path.display()
                    )?;
                }
            }
            return Err(message.into());
        }
    }
    Ok(())
}

/// Validate every input before updating release files.
fn bump(root: &Path, new: &str) -> Result<()> {
    let mut manifest = FileChange::read(root.join("Cargo.toml"))?;
    let (prefix, section) = manifest
        .original
        .split_once("[workspace.package]")
        .ok_or("Workspace package not found.")?;
    let old = section
        .lines()
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
        })
        .ok_or("Workspace version not found.")?;
    if !newer(new, old)? {
        return Err(format!("The new version must be greater than {old}.").into());
    }
    let mut files = versioned_files(root)?;
    files.extend([
        root.join("docs/schema/deploy.schema.json"),
        root.join("docs/website/public/deploy.schema.json"),
        root.join("docs/schema/protocol.schema.json"),
    ]);
    let mut changes = Vec::new();
    let old_tag = format!("v{old}");
    let new_tag = format!("v{new}");
    for path in files {
        let mut change = FileChange::read(path)?;
        if !change.original.contains(&old_tag) {
            return Err(format!("{} does not contain v{old}.", change.path.display()).into());
        }
        change.updated = change.original.replace(&old_tag, &new_tag);
        changes.push(change);
    }
    manifest.updated = format!(
        "{prefix}[workspace.package]{}",
        section.replacen(
            &format!("version = \"{old}\""),
            &format!("version = \"{new}\""),
            1,
        )
    );
    let mut lock = FileChange::read(root.join("Cargo.lock"))?;
    lock.updated = lockfile(&lock.original, old, new)?;
    changes.push(lock);
    let message =
        format!("Updated shipdeck from {old} to {new}. Add release notes to CHANGELOG.md.");
    changes.push(manifest);
    apply(&changes)?;
    println!("{message}");
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let new = args.next().ok_or("Usage: just version MAJOR.MINOR.PATCH")?;
    if args.next().is_some() {
        return Err("Usage: just version MAJOR.MINOR.PATCH".into());
    }
    let root = std::env::current_dir()?;
    bump(&root, &new)
}
