//! Selects backup files separately for each app in a shared release.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Result, bail};

use crate::config::glob::{Rules, Verdict};
use crate::config::monorepo::{AppDefinition, MonorepoConfig};
use crate::config::{BackupFiles, Pipeline};
use crate::remote::{Picked, ReleaseFile};

/// Applies the global and per-app backup settings to a release listing.
///
/// # Errors
/// Returns an error when an app is unresolved or a backup pattern is invalid.
pub fn pick_apps(
    config: &MonorepoConfig,
    listing: &[ReleaseFile],
    deployed: &[String],
    own: &[String],
) -> Result<Picked> {
    let global: Pipeline = serde_json::from_value(config.pipeline.clone())?;
    let mut global = Scope::new(&global.backup.files)?;
    let mut apps = BTreeMap::new();
    let mut shared = Vec::new();
    for (name, definition) in &config.apps {
        let AppDefinition::Inline(app) = definition else {
            bail!("app {name} config file has not been loaded");
        };
        let pipeline = app.pipeline(&config.pipeline)?;
        apps.insert(name.as_str(), Scope::new(&pipeline.backup.files)?);
        shared.extend(app.shared.iter().map(|path| format!("apps/{name}/{path}")));
    }
    let deployed: BTreeSet<&str> = deployed.iter().map(String::as_str).collect();
    let mut sorted: Vec<&ReleaseFile> = listing.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut paths = Vec::new();
    for file in sorted {
        let path = file.path.as_str();
        if shared
            .iter()
            .any(|folder| Path::new(path).starts_with(folder))
        {
            continue;
        }
        if own.iter().any(|item| item == path)
            || path == crate::config::monorepo::RELEASE_METADATA
            || path == crate::release::MANIFEST_FILE
        {
            paths.push(file.path.clone());
            continue;
        }
        let app_path = path
            .strip_prefix("apps/")
            .and_then(|rest| rest.split_once('/'));
        let wanted = if let Some((name, relative)) = app_path {
            if let Some(scope) = apps.get_mut(name) {
                scope.include(relative, file.is_dir, deployed.contains(path))
            } else {
                global.include(path, file.is_dir, deployed.contains(path))
            }
        } else if path == "apps"
            || path
                .strip_prefix("apps/")
                .is_some_and(|name| apps.contains_key(name))
        {
            true
        } else {
            global.include(path, file.is_dir, deployed.contains(path))
        };
        if wanted {
            paths.push(file.path.clone());
        }
    }
    let mut unmatched = global.unmatched("global");
    for (name, scope) in &apps {
        unmatched.extend(scope.unmatched(name));
    }
    Ok(Picked { paths, unmatched })
}

/// Tracks backup pattern matches within one app or the global release root.
struct Scope {
    rules: Rules,
    hits: Vec<usize>,
    root: Verdict,
    folders: BTreeMap<String, Verdict>,
    deployed: bool,
}

impl Scope {
    fn new(files: &BackupFiles) -> Result<Self> {
        let rules = Rules::parse(&files.include, &files.exclude).map_err(anyhow::Error::msg)?;
        let mut hits = vec![0; rules.include.len()];
        let root = rules.judge(Verdict::default(), "", true, &mut hits);
        Ok(Self {
            rules,
            hits,
            root,
            folders: BTreeMap::new(),
            deployed: files.deployed,
        })
    }

    fn include(&mut self, path: &str, is_dir: bool, deployed: bool) -> bool {
        let parent = path
            .rsplit_once('/')
            .and_then(|(parent, _)| self.folders.get(parent))
            .copied()
            .unwrap_or(self.root);
        let verdict = self.rules.judge(parent, path, is_dir, &mut self.hits);
        if is_dir {
            self.folders.insert(path.to_string(), verdict);
        }
        ((self.deployed && deployed) || verdict.included()) && !verdict.excluded()
    }

    fn unmatched(&self, scope: &str) -> Vec<String> {
        self.rules
            .include
            .iter()
            .zip(&self.hits)
            .filter(|(pattern, count)| !pattern.negated && **count == 0)
            .map(|(pattern, _)| format!("{scope}: {}", pattern.text))
            .collect()
    }
}

#[cfg(test)]
#[path = "backup_apps_test.rs"]
mod tests;
