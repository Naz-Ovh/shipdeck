//! Finding the deployables under a deploy root.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Deployable, FILE_NAME};

/// What a scan of the deploy root found.
#[derive(Debug, Default)]
pub struct Discovery {
    pub deployables: Vec<Deployable>,
    /// Folders that have a `deploy.json` that could not be used, with the reason.
    pub broken: Vec<(PathBuf, String)>,
}

/// Every direct subfolder of `root` that holds a `deploy.json`, sorted by name.
pub fn discover(root: &Path) -> Result<Discovery> {
    let entries = std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir() && path.join(FILE_NAME).is_file())
        .collect();
    folders.sort();

    let mut found = Discovery::default();
    for folder in folders {
        match Deployable::load(&folder) {
            Ok(deployable) => found.deployables.push(deployable),
            Err(err) => found.broken.push((folder, format!("{err:#}"))),
        }
    }
    Ok(found)
}
