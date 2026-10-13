//! Release names (`v0.3.1-ab12cd3-20261014.153012`) and the `release.json`
//! each release carries.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

const STAMP_FORMAT: &str = "%Y%m%d.%H%M%S";

/// The parts a release id is made of. Ids sort by `stamp`, never by text:
/// `v0.10.0` must count as newer than `v0.9.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseName {
    pub version: String,
    pub git: String,
    pub dirty: bool,
    /// Build time in UTC.
    pub stamp: NaiveDateTime,
}

impl ReleaseName {
    pub fn new(version: &str, git: &str, dirty: bool, built_at: DateTime<Utc>) -> Self {
        // Drop sub-second precision so the id round-trips through `parse`.
        let stamp =
            NaiveDateTime::parse_from_str(&built_at.format(STAMP_FORMAT).to_string(), STAMP_FORMAT)
                .expect("formatted stamp parses");
        Self {
            // Parsing reads from the right, so a version may keep its dashes
            // (`1.0.0-beta.2`); the git part may not.
            version: sanitize(version, "._+-"),
            git: sanitize(git, "._+"),
            dirty,
            stamp,
        }
    }

    /// `v<version>-<git>[-dirty]-<YYYYMMDD.HHMMSS>`
    pub fn id(&self) -> String {
        let dirty = if self.dirty { "-dirty" } else { "" };
        format!(
            "v{}-{}{}-{}",
            self.version,
            self.git,
            dirty,
            self.stamp.format(STAMP_FORMAT)
        )
    }

    /// `v0.3.1 (ab12cd3)`, for the unit's description and the screens.
    pub fn tag(&self) -> String {
        if self.dirty {
            format!("v{} ({}, uncommitted changes)", self.version, self.git)
        } else {
            format!("v{} ({})", self.version, self.git)
        }
    }

    /// Reads an id back. Anything shipdeck did not name returns `None` and is
    /// left alone on the server.
    pub fn parse(id: &str) -> Option<Self> {
        if !is_safe_id(id) {
            return None;
        }
        let rest = id.strip_prefix('v')?;
        let (rest, stamp) = rest.rsplit_once('-')?;
        let stamp = NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT).ok()?;
        let (rest, dirty) = match rest.strip_suffix("-dirty") {
            Some(rest) => (rest, true),
            None => (rest, false),
        };
        let (version, git) = rest.rsplit_once('-')?;
        if version.is_empty() || git.is_empty() {
            return None;
        }
        Some(Self {
            version: version.to_string(),
            git: git.to_string(),
            dirty,
            stamp,
        })
    }
}

/// Ids go into remote shell commands, so only these characters are allowed.
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
}

fn sanitize(part: &str, allowed: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || allowed.contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `release.json`: what was deployed, by whom and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub git: String,
    pub dirty: bool,
    pub built_at: String,
    pub deployed_by: String,
}

impl Manifest {
    pub fn new(name: &ReleaseName, deployed_by: &str) -> Self {
        Self {
            id: name.id(),
            version: name.version.clone(),
            git: name.git.clone(),
            dirty: name.dirty,
            built_at: name.stamp.and_utc().to_rfc3339(),
            deployed_by: deployed_by.to_string(),
        }
    }
}

#[cfg(test)]
#[path = "release_test.rs"]
mod tests;
