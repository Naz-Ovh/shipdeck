//! The `<deploy>` argument: a deploy name from the root file, or a deploy file.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use shipdeck_engine::config;

/// What a command was told to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A deploy name from the root file.
    Name(String),
    /// A deploy file, by its path on the command line.
    File(PathBuf),
}

impl Target {
    /// The path this target names, when it names a file.
    #[must_use]
    pub fn file(&self) -> Option<&Path> {
        match self {
            Target::File(path) => Some(path),
            Target::Name(_) => None,
        }
    }

    /// The path resolved from the project folder: a relative path starts
    /// there, as `git -C` makes it.
    #[must_use]
    pub fn resolved(&self, root: &Path) -> Self {
        match self {
            Target::Name(name) => Target::Name(name.clone()),
            Target::File(path) => Target::File(root.join(path)),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Name(name) => f.write_str(name),
            Target::File(path) => write!(f, "{}", path.display()),
        }
    }
}

impl FromStr for Target {
    type Err = String;

    /// Only the ending decides: `.yaml` or `.yml` is a deploy file, and
    /// anything else must be a deploy name.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if config::is_deploy_file(text) {
            return Ok(Target::File(PathBuf::from(text)));
        }
        if config::is_deploy_name(text) {
            return Ok(Target::Name(text.to_string()));
        }
        Err(format!(
            "{text:?} is not a deploy: a deploy file's name ends in .yaml or .yml, \
             and a deploy name is letters, digits, `-` and `_`"
        ))
    }
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
