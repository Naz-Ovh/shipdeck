//! Finding the deployables under a deploy root.

use std::path::Path;

use anyhow::{Context, Result};

use super::problem::{FileProblems, Problem};
use super::{Deployable, FILE_NAMES, LEGACY_FILE_NAME, config_file};

/// What a scan of the deploy root found.
#[derive(Debug, Default)]
pub struct Discovery {
    pub deployables: Vec<Deployable>,
    /// Config files that cannot be used, with everything wrong in each.
    /// shipdeck does not start while there are any.
    pub problems: Vec<FileProblems>,
}

/// Every direct subfolder of `root` that holds a `deploy.yaml` (or
/// `deploy.yml`), sorted by name. A folder with only a `deploy.json` is a
/// problem, so it is never skipped without a word.
pub fn discover(root: &Path) -> Result<Discovery> {
    let entries = std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
    let mut folders: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_dir() && (config_file(path).is_some() || path.join(LEGACY_FILE_NAME).is_file())
        })
        .collect();
    folders.sort();

    let mut found = Discovery::default();
    for folder in folders {
        if config_file(&folder).is_none() {
            found.problems.push(FileProblems::without_source(
                folder.join(LEGACY_FILE_NAME),
                Problem::new(format!(
                    "shipdeck reads {yaml} now, not {LEGACY_FILE_NAME}",
                    yaml = FILE_NAMES[0]
                ))
                .help("rename it to deploy.yaml and move its fields as the README's \"From 0.2\" shows"),
            ));
            continue;
        }
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
