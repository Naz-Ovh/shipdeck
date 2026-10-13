//! `deploy.json`: what one deployable is, how it is built and where it goes.

mod discover;

pub use discover::{Discovery, discover};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::release::MANIFEST_FILE;
use crate::ssh::Endpoint;

/// The file that marks a folder as a deployable.
pub const FILE_NAME: &str = "deploy.json";
/// Optional file next to `deploy.json` that replaces the built-in systemd unit.
pub const UNIT_TEMPLATE: &str = "service.template";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployConfig {
    pub name: String,
    pub ssh: SshTarget,
    pub build: BuildConfig,
    pub remote: RemoteConfig,
    #[serde(default)]
    pub releases: Retention,
    #[serde(default)]
    pub bundle: Vec<String>,
    #[serde(default)]
    pub health: HealthConfig,
}

/// The `ssh` block. `host` may be a `~/.ssh/config` alias; `port` and
/// `user` fall back to that alias, then to 22 and nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshTarget {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    #[serde(default = "default_cwd")]
    pub cwd: String,
    pub command: Vec<String>,
    pub binary: String,
    #[serde(default)]
    pub version_from: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteConfig {
    pub dir: String,
    pub service: String,
    /// Arguments after the binary in `ExecStart=`.
    #[serde(default)]
    pub args: Vec<String>,
    /// The user the service runs as. Defaults to the SSH user.
    #[serde(default)]
    pub run_as: Option<String>,
    /// Extra `Environment=` lines in the unit.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    #[serde(default = "default_open")]
    pub open: usize,
    #[serde(default = "default_compressed")]
    pub compressed: usize,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            open: default_open(),
            compressed: default_compressed(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthConfig {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            url: None,
            timeout_secs: default_timeout(),
        }
    }
}

fn default_cwd() -> String {
    ".".into()
}
fn default_open() -> usize {
    5
}
fn default_compressed() -> usize {
    15
}
fn default_timeout() -> u64 {
    20
}

/// One folder under the deploy root, with its parsed `deploy.json`.
#[derive(Debug, Clone)]
pub struct Deployable {
    pub folder: PathBuf,
    /// The folder name, used as a short handle in the UI.
    pub slug: String,
    pub config: DeployConfig,
    /// `config.ssh` with `~/.ssh/config` applied.
    pub endpoint: Endpoint,
}

impl Deployable {
    pub fn load(folder: &Path) -> Result<Self> {
        let path = folder.join(FILE_NAME);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let config: DeployConfig =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.validate()?;
        let endpoint = Endpoint::resolve(&config.ssh)?;
        let slug = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.display().to_string());
        Ok(Self {
            folder: folder.to_path_buf(),
            slug,
            config,
            endpoint,
        })
    }

    /// Where the build command runs.
    pub fn build_dir(&self) -> PathBuf {
        self.folder.join(&self.config.build.cwd)
    }

    /// The binary the build produces.
    pub fn binary_path(&self) -> PathBuf {
        self.build_dir().join(&self.config.build.binary)
    }

    /// The binary's file name, which is also its name on the server.
    pub fn binary_name(&self) -> String {
        Path::new(&self.config.build.binary)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.config.build.binary.clone())
    }

    /// The `Cargo.toml` whose version names a release.
    pub fn version_file(&self) -> PathBuf {
        match &self.config.build.version_from {
            Some(path) => self.folder.join(path),
            None => self.build_dir().join("Cargo.toml"),
        }
    }

    pub fn unit_template_path(&self) -> PathBuf {
        self.folder.join(UNIT_TEMPLATE)
    }

    /// The user the service runs as.
    pub fn run_as(&self) -> &str {
        self.config
            .remote
            .run_as
            .as_deref()
            .unwrap_or(&self.endpoint.user)
    }

    /// Each `bundle` entry with whether it exists locally.
    pub fn bundle_status(&self) -> Vec<(String, bool)> {
        self.config
            .bundle
            .iter()
            .map(|entry| (entry.clone(), self.folder.join(entry).exists()))
            .collect()
    }

    /// Checks every `bundle` entry exists and can be read, and names all the
    /// problems at once rather than the first.
    pub fn check_bundle(&self) -> Result<()> {
        let mut problems = Vec::new();
        for entry in &self.config.bundle {
            let path = self.folder.join(entry);
            let readable = match std::fs::metadata(&path) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    problems.push(format!("{entry}: missing"));
                    continue;
                }
                Err(err) => Err(err),
                Ok(meta) if meta.is_dir() => std::fs::read_dir(&path).map(|_| ()),
                Ok(_) => std::fs::File::open(&path).map(|_| ()),
            };
            if let Err(err) = readable {
                problems.push(format!("{entry}: {err}"));
            }
        }
        if problems.is_empty() {
            return Ok(());
        }
        bail!(
            "{} of {} bundle entries cannot be shipped from {}: {}",
            problems.len(),
            self.config.bundle.len(),
            self.folder.display(),
            problems.join("; ")
        )
    }
}

/// A `bundle` entry as it is stored in the release: `./files/` → `files`.
/// Empty for an entry that names the deployable folder itself.
pub fn bundle_path(entry: &str) -> String {
    Path::new(entry)
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl DeployConfig {
    /// Rejects anything that would be unsafe or ambiguous once it reaches a
    /// remote shell or a systemd unit.
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "`name` is empty");
        ensure!(
            !self.name.contains(['\n', '\r']),
            "`name` must be a single line"
        );

        ensure!(!self.ssh.host.is_empty(), "`ssh.host` is empty");
        if let Some(user) = &self.ssh.user {
            ensure!(
                is_plain_name(user),
                "`ssh.user` may only use letters, digits, `.`, `_` and `-`"
            );
        }

        ensure!(!self.build.command.is_empty(), "`build.command` is empty");
        ensure!(!self.build.binary.is_empty(), "`build.binary` is empty");

        let dir = &self.remote.dir;
        ensure!(
            dir.starts_with('/') && dir.len() > 1,
            "`remote.dir` must be an absolute path such as /opt/my-app"
        );
        ensure!(
            dir.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c)),
            "`remote.dir` may only use letters, digits, `.`, `_`, `-` and `/`"
        );
        ensure!(
            !dir.split('/').any(|part| part == ".."),
            "`remote.dir` must not contain `..`"
        );

        let service = &self.remote.service;
        ensure!(
            !service.ends_with(".service"),
            "`remote.service` is the name without `.service`"
        );
        ensure!(
            !service.is_empty()
                && service
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._@-".contains(c)),
            "`remote.service` may only use letters, digits, `.`, `_`, `@` and `-`"
        );
        if let Some(user) = &self.remote.run_as {
            ensure!(
                is_plain_name(user),
                "`remote.run_as` may only use letters, digits, `.`, `_` and `-`"
            );
        }
        for arg in &self.remote.args {
            ensure!(
                !arg.contains(['"', '\\', '\n', '\r']),
                "`remote.args` entries must not contain quotes, backslashes or newlines: {arg:?}"
            );
        }
        for (key, value) in &self.remote.env {
            ensure!(
                !key.is_empty()
                    && !key.starts_with(|c: char| c.is_ascii_digit())
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "`remote.env` key {key:?} is not a valid variable name"
            );
            ensure!(
                !value.contains(['"', '\\', '\n', '\r']),
                "`remote.env` value of {key} must not contain quotes, backslashes or newlines"
            );
        }

        ensure!(
            self.releases.open >= 1,
            "`releases.open` must be at least 1"
        );

        self.validate_bundle()?;

        if let Some(url) = &self.health.url {
            ensure!(
                url.starts_with("http://") || url.starts_with("https://"),
                "`health.url` must start with http:// or https://"
            );
            ensure!(
                !url.contains(['\'', '\n', '\r', ' ']),
                "`health.url` must not contain quotes, spaces or newlines"
            );
        }
        ensure!(
            self.health.timeout_secs >= 3,
            "`health.timeout_secs` must be at least 3"
        );
        Ok(())
    }
}

impl DeployConfig {
    /// Every entry names a file or folder inside the deployable folder, once,
    /// without clashing with the files every release carries.
    fn validate_bundle(&self) -> Result<()> {
        let binary = Path::new(&self.build.binary)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let unit = format!("{}.service", self.remote.service);
        let reserved = [binary.as_str(), MANIFEST_FILE, unit.as_str()];

        let mut seen: Vec<(String, &str)> = Vec::new();
        for entry in &self.bundle {
            let path = Path::new(entry);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                bail!("`bundle` entry {entry:?} must be a path inside the deployable folder");
            }
            let stored = bundle_path(entry);
            if stored.is_empty() {
                bail!(
                    "`bundle` entry {entry:?} names the whole deployable folder, \
                     which would ship deploy.json too; list the files instead"
                );
            }
            if reserved.contains(&stored.as_str()) {
                bail!("`bundle` entry {entry:?} would replace the release's own {stored}");
            }
            for (other, original) in &seen {
                if *other == stored {
                    bail!("`bundle` lists {stored:?} twice ({original:?} and {entry:?})");
                }
                let (outer, inner) = if stored.len() < other.len() {
                    (&stored, other)
                } else {
                    (other, &stored)
                };
                if inner.starts_with(&format!("{outer}/")) {
                    bail!(
                        "`bundle` entries {original:?} and {entry:?} overlap; {outer:?} already includes {inner:?}"
                    );
                }
            }
            seen.push((stored, entry));
        }
        Ok(())
    }
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
