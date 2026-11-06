//! Loading either supported deployment shape without changing single-app YAML.

use std::path::{Path, PathBuf};

use super::monorepo::MonorepoConfig;
use super::{Deployable, FILE_NAMES, FileProblems, Problem};
use crate::ssh::Endpoint;

/// A deployment read from one folder.
#[derive(Debug, Clone)]
pub enum Deployment {
    /// The existing single-service config.
    Single(Box<Deployable>),
    /// Several apps sharing one release.
    Monorepo(Box<MonorepoDeployment>),
}

/// A monorepo config and its connection settings.
#[derive(Debug, Clone)]
pub struct MonorepoDeployment {
    pub folder: PathBuf,
    pub slug: String,
    pub config: MonorepoConfig,
    pub endpoint: Endpoint,
}

impl Deployment {
    /// Loads a deployment and resolves its SSH endpoint.
    ///
    /// # Errors
    /// Returns the config problems when the deployment cannot be loaded.
    pub fn load(folder: &Path) -> Result<Self, FileProblems> {
        let files: Vec<_> = FILE_NAMES
            .iter()
            .map(|name| folder.join(name))
            .filter(|path| path.is_file())
            .collect();
        let [path] = files.as_slice() else {
            return Deployable::load(folder).map(|deployment| Self::Single(Box::new(deployment)));
        };
        let text = std::fs::read_to_string(path).map_err(|err| {
            FileProblems::without_source(
                path.clone(),
                Problem::new(format!("cannot read it: {err}")),
            )
        })?;
        let root = match super::yaml::parse(&text) {
            Ok(root) => root,
            Err(problem) => {
                return Err(FileProblems {
                    path: path.clone(),
                    source: Some(text),
                    problems: vec![problem],
                });
            }
        };
        if root.find(&["apps"]).is_none() {
            return Deployable::from_text(folder, path, text)
                .map(|deployment| Self::Single(Box::new(deployment)));
        }
        let config = MonorepoConfig::from_text(folder, path, &text)?;
        let endpoint = Endpoint::resolve(&config.ssh).map_err(|err| {
            FileProblems::without_source(path.clone(), Problem::new(format!("{err:#}")))
        })?;
        let slug = folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        Ok(Self::Monorepo(Box::new(MonorepoDeployment {
            folder: folder.to_path_buf(),
            slug,
            config,
            endpoint,
        })))
    }
}

#[cfg(test)]
#[path = "deployment_test.rs"]
mod tests;
