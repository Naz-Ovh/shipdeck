//! Installed agent versions and the rules for connecting to them.

use std::fmt;

use anyhow::{Result, bail};

/// A `major.minor.patch` release version. A pre-release suffix is ignored
/// here, because only the three numbers take part in the compatibility rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Reads `1.2.3` or `1.2.3-rc.1`.
    ///
    /// # Errors
    ///
    /// Fails when a part is missing or is not a number.
    pub fn parse(text: &str) -> Result<Self> {
        let core = text.trim().split('-').next().unwrap_or_default();
        let mut parts = core.split('.');
        let major = number(parts.next(), text)?;
        let minor = number(parts.next(), text)?;
        let patch = number(parts.next(), text)?;
        if parts.next().is_some() {
            bail!("{text:?} has more than three version numbers");
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

fn number(part: Option<&str>, whole: &str) -> Result<u64> {
    let part = part.unwrap_or_default();
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("{whole:?} is not a major.minor.patch version");
    }
    part.parse()
        .map_err(|_| anyhow::anyhow!("{whole:?} has a version number that is too large"))
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One agent installation that passed the ownership and version probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    /// The exact version text in the directory and printed by the binary.
    pub label: String,
    /// The full executable path, including the versioned directory.
    pub path: String,
}

/// What the connecting shipdeck must do about the installed agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Start this installation; a different minor or patch is compatible.
    Use(Installed),
    /// The agent's major is newer; refuse and name it.
    Refuse(Installed),
    /// Only older major versions are installed; clear them and install.
    Replace,
    /// Nothing valid is installed.
    Install,
}

/// Applies the major-version rules to one server's installations.
///
/// A newer agent major blocks the connection even when a same-major install
/// also exists, so keeping several versioned directories cannot bypass the
/// rule. Within the same major, the newest installation is used.
#[must_use]
pub fn choose(cli: &Version, installed: &[Installed]) -> Action {
    let newest = installed.iter().max_by_key(|candidate| candidate.version);
    if let Some(newest) = newest
        && newest.version.major > cli.major
    {
        return Action::Refuse(newest.clone());
    }
    let same_major = installed
        .iter()
        .filter(|candidate| candidate.version.major == cli.major)
        .max_by_key(|candidate| candidate.version);
    if let Some(best) = same_major {
        return Action::Use(best.clone());
    }
    if installed.is_empty() {
        Action::Install
    } else {
        Action::Replace
    }
}

#[cfg(test)]
#[path = "version_test.rs"]
mod tests;
