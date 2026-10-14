//! Finding the deployables under a deploy root.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Deployable, FILE_NAMES, LEGACY_FILE_NAME, config_file};

/// What a scan of the deploy root found.
#[derive(Debug, Default)]
pub struct Discovery {
    pub deployables: Vec<Deployable>,
    /// Folders with a config that could not be used, with the reason.
    pub broken: Vec<(PathBuf, String)>,
}

/// Every direct subfolder of `root` that holds a `deploy.yaml` (or
/// `deploy.yml`), sorted by name. A folder with only a `deploy.json` is
/// listed as broken, so it is never skipped without a word.
pub fn discover(root: &Path) -> Result<Discovery> {
    let entries = std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_dir() && (config_file(path).is_some() || path.join(LEGACY_FILE_NAME).is_file())
        })
        .collect();
    folders.sort();

    let mut found = Discovery::default();
    for folder in folders {
        if config_file(&folder).is_none() {
            found.broken.push((
                folder,
                format!(
                    "shipdeck now reads {yaml}, not {LEGACY_FILE_NAME}. JSON is valid YAML, \
                     so renaming the file to {yaml} works; the README lists the new fields",
                    yaml = FILE_NAMES[0]
                ),
            ));
            continue;
        }
        match Deployable::load(&folder) {
            Ok(deployable) => found.deployables.push(deployable),
            Err(err) => found.broken.push((folder, format!("{err:#}"))),
        }
    }
    Ok(found)
}
