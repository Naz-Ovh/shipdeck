//! The values shipdeck reads for itself: the real environment over a `.env`
//! file. They feed `${NAME}` in a deploy, `SHIPDECK_SSH_*` and the login
//! variables, and nothing else; commands never see them.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// The file read when `--env-file` names none.
pub const DEFAULT_FILE: &str = ".env";

/// Named values: what the real environment sets, else what the file sets.
#[derive(Debug, Clone, Default)]
pub struct Env {
    values: BTreeMap<String, String>,
}

impl Env {
    /// No values at all.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The values of the real environment, over the env file.
    ///
    /// # Errors
    /// Returns an error when a named file is missing, or a file cannot be read.
    pub fn load(folder: &Path, file: Option<&Path>) -> Result<Self> {
        let real = std::env::vars_os().filter_map(|(name, value)| {
            Some((name.into_string().ok()?, value.into_string().ok()?))
        });
        Self::load_over(folder, file, real)
    }

    /// The same, with `real` standing for the real environment.
    ///
    /// `file` is `--env-file`: a relative path starts from `folder`. Without
    /// it, `folder/.env` is read when it exists. A value that is empty counts
    /// as not set.
    ///
    /// # Errors
    /// Returns an error when a named file is missing, or a file cannot be read.
    pub fn load_over(
        folder: &Path,
        file: Option<&Path>,
        real: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        let mut values = BTreeMap::new();
        let (path, named) = match file {
            Some(file) => (folder.join(file), true),
            None => (folder.join(DEFAULT_FILE), false),
        };
        if path.is_file() {
            read_file(&path, &mut values)?;
        } else if named {
            bail!("the env file {} does not exist", path.display());
        }
        for (name, value) in real {
            if !value.is_empty() {
                values.insert(name, value);
            }
        }
        Ok(Self { values })
    }

    /// The values `pairs` hold, as if the real environment had them.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let values = pairs
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Self { values }
    }

    /// The value of `name`, when it is set and not empty.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

/// Reads `NAME=value` lines; the first of a repeated name stays.
fn read_file(path: &Path, values: &mut BTreeMap<String, String>) -> Result<()> {
    let lines = dotenvy::from_path_iter(path)
        .with_context(|| format!("cannot read the env file {}", path.display()))?;
    for item in lines {
        let (name, value) = item.map_err(|err| match err {
            // The error text holds the line, and a line may hold a secret.
            dotenvy::Error::LineParse(_, _) => anyhow::anyhow!(
                "the env file {} has a line that is not NAME=value",
                path.display()
            ),
            other => anyhow::Error::new(other)
                .context(format!("cannot read the env file {}", path.display())),
        })?;
        values.entry(name).or_insert(value);
    }
    Ok(())
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
