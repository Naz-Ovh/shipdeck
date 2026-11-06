//! Update the workspace version and regenerate its published schemas.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

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

/// Run Cargo and write the generated schema only after it succeeds.
fn schema(root: &Path, args: &[&str], paths: &[&str]) -> Result<()> {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Schema generation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    for path in paths {
        fs::write(root.join(path), &output.stdout)?;
    }
    Ok(())
}

/// Apply a version change, restoring the original files if a command fails.
fn bump(root: &Path, new: &str) -> Result<()> {
    let manifest = root.join("Cargo.toml");
    let text = fs::read_to_string(&manifest)?;
    let section = text
        .split_once("[workspace.package]\n")
        .ok_or("Workspace package not found.")?
        .1;
    let old = section
        .lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
        })
        .ok_or("Workspace version not found.")?;
    if !newer(new, old)? {
        return Err(format!("The new version must be greater than {old}.").into());
    }
    let files = versioned_files(root)?;
    let mut changes = Vec::new();
    for path in files {
        let content = fs::read_to_string(&path)?;
        if !content.contains(&format!("v{old}")) {
            return Err(format!("{} does not contain v{old}.", path.display()).into());
        }
        changes.push((
            path,
            content.replace(&format!("v{old}"), &format!("v{new}")),
        ));
    }
    changes.push((
        manifest,
        text.replacen(
            &format!("[workspace.package]\nversion = \"{old}\""),
            &format!("[workspace.package]\nversion = \"{new}\""),
            1,
        ),
    ));
    let mut affected: Vec<_> = changes.iter().map(|(path, _)| path.clone()).collect();
    affected.extend(
        [
            "Cargo.lock",
            "docs/schema/deploy.schema.json",
            "docs/website/public/deploy.schema.json",
            "docs/schema/protocol.schema.json",
        ]
        .iter()
        .map(|path| root.join(path)),
    );
    let originals: Vec<_> = affected
        .iter()
        .map(|path| Ok((path, fs::read(path)?)))
        .collect::<Result<_>>()?;
    let result = (|| -> Result<()> {
        for (path, content) in &changes {
            fs::write(path, content)?;
        }
        schema(
            root,
            &["run", "-q", "-p", "shipdeck", "--", "--schema"],
            &[
                "docs/schema/deploy.schema.json",
                "docs/website/public/deploy.schema.json",
            ],
        )?;
        schema(
            root,
            &[
                "run",
                "-q",
                "-p",
                "shipdeck-protocol",
                "--example",
                "schema",
            ],
            &["docs/schema/protocol.schema.json"],
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        for (path, content) in originals {
            fs::write(path, content)?;
        }
        return Err(error);
    }
    println!("Updated shipdeck from {old} to {new}. Add release notes to CHANGELOG.md.");
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let new = args.next().ok_or("Usage: just version MAJOR.MINOR.PATCH")?;
    if args.next().is_some() {
        return Err("Usage: just version MAJOR.MINOR.PATCH".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    bump(&root, &new)
}
