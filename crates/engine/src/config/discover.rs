//! Finding the deployables under a deploy root.

use std::path::Path;

use anyhow::{Context, Result};

use super::problem::FileProblems;
use super::{Deployable, config_file};

/// What a scan of the deploy root found.
#[derive(Debug, Default)]
pub struct Discovery {
    pub deployables: Vec<Deployable>,
    /// Config files that cannot be used, with everything wrong in each.
    /// shipdeck does not start while there are any.
    pub problems: Vec<FileProblems>,
}

/// Every direct subfolder of `root` that holds a `deploy.yaml` (or
/// `deploy.yml`), sorted by name.
pub fn discover(root: &Path) -> Result<Discovery> {
    let entries = std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
    let mut folders: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir() && config_file(path).is_some())
        .collect();
    folders.sort();

    let mut found = Discovery::default();
    for folder in folders {
        match Deployable::load(&folder) {
            Ok(deployable) => found.deployables.push(deployable),
            Err(problems) => found.problems.push(problems),
        }
    }
    Ok(found)
}

#[cfg(test)]
#[path = "discover_test.rs"]
mod tests;
