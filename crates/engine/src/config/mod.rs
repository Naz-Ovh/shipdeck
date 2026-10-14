//! `deploy.yaml`: what one deployable is, how it is built and where it goes.

mod discover;

pub use discover::{Discovery, discover};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;

use crate::release::MANIFEST_FILE;
use crate::ssh::Endpoint;

/// The files that mark a folder as a deployable; a folder may have one of them.
pub const FILE_NAMES: [&str; 2] = ["deploy.yaml", "deploy.yml"];
/// What shipdeck 0.1 read. Found alone, it is reported instead of skipped.
pub const LEGACY_FILE_NAME: &str = "deploy.json";
/// Optional file next to `deploy.yaml` that replaces the built-in systemd unit.
pub const UNIT_TEMPLATE: &str = "service.template";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployConfig {
    pub name: String,
    pub ssh: SshTarget,
    #[serde(default)]
    pub hooks: Hooks,
    #[serde(default)]
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

/// Shell commands around the automated steps. Each runs with `sh -c`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// On this machine in `build.cwd`, before anything else. A failure stops
    /// the deploy before the server is touched.
    #[serde(default)]
    pub before: Vec<String>,
    /// On the server inside the new release folder, after it is unpacked and
    /// before it goes live (`npm ci --omit=dev`, migrations). A failure
    /// removes the release; the live one keeps running.
    #[serde(default)]
    pub server: Vec<String>,
    /// On this machine in `build.cwd`, once the new release is live. A
    /// failure is only a warning.
    #[serde(default)]
    pub after: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    #[serde(default = "default_cwd")]
    pub cwd: String,
    /// Optional: an app that ships its files as they are needs no build.
    #[serde(default)]
    pub command: Option<CommandLine>,
    /// A compiled program, relative to `cwd`. It lands at the release root
    /// and is checked against the server's CPU and glibc.
    #[serde(default)]
    pub binary: Option<String>,
    /// Build output to ship (`dist`, `package.json`), relative to `cwd`.
    /// Each keeps its path inside the release.
    #[serde(default)]
    pub files: Vec<String>,
    /// The file whose version names a release, relative to the deployable
    /// folder. Found in `cwd` when unset; see `Deployable::version_file`.
    #[serde(default)]
    pub version_from: Option<String>,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            cwd: default_cwd(),
            command: None,
            binary: None,
            files: Vec::new(),
            version_from: None,
        }
    }
}

/// A command to run on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum CommandLine {
    /// `npm ci && npm run build`: runs with `sh -c`, so `&&`, pipes and
    /// variables work.
    Shell(String),
    /// `[cargo, build, --release]`: runs as given, without a shell.
    Args(Vec<String>),
}

impl CommandLine {
    /// How the command reads in the log and on the screen.
    pub fn display(&self) -> String {
        match self {
            Self::Shell(line) => line.trim().to_string(),
            Self::Args(args) => args.join(" "),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteConfig {
    pub dir: String,
    pub service: String,
    /// What the service runs, from the release folder: `node dist/main.js`.
    /// Defaults to the binary.
    #[serde(default)]
    pub start: Option<String>,
    /// Arguments after the start command in `ExecStart=`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Programs the server must have (`node`, `npm`), checked at preflight.
    #[serde(default)]
    pub requires: Vec<String>,
    /// The user the service runs as. Defaults to the SSH user.
    #[serde(default)]
    pub run_as: Option<String>,
    /// Extra `Environment=` lines in the unit.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// A file in the release whose `KEY=value` lines become the service's
    /// environment (`EnvironmentFile=`), e.g. `.env`.
    #[serde(default)]
    pub env_file: Option<String>,
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

/// Files whose version names a release, looked for in `build.cwd` in this
/// order when `build.version_from` is not set.
pub const VERSION_FILES: [&str; 4] = ["Cargo.toml", "package.json", "pyproject.toml", "VERSION"];

/// Reads and checks the text of a `deploy.yaml`.
pub fn parse(text: &str) -> Result<DeployConfig> {
    let config: DeployConfig = serde_saphyr::from_str(text)
        .map_err(|err| anyhow!(err.render_with_formatter(&serde_saphyr::UserMessageFormatter)))?;
    config.validate()?;
    Ok(config)
}

/// The config file in `folder`, if it has one.
pub fn config_file(folder: &Path) -> Option<PathBuf> {
    FILE_NAMES
        .iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
}

/// Where the program in `remote.start` (or the binary) comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// `node`: looked up on the server's `PATH` at preflight.
    Tool(String),
    /// `/usr/bin/node`: used as written.
    Absolute(String),
    /// `./my-api`, `bin/server`: a file inside the release.
    InRelease(String),
}

/// One folder under the deploy root, with its parsed `deploy.yaml`.
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
        let found: Vec<PathBuf> = FILE_NAMES
            .iter()
            .map(|name| folder.join(name))
            .filter(|path| path.is_file())
            .collect();
        let path = match found.as_slice() {
            [one] => one,
            [] => bail!("{} has no {}", folder.display(), FILE_NAMES[0]),
            _ => bail!(
                "{} has both {} and {}; keep one",
                folder.display(),
                FILE_NAMES[0],
                FILE_NAMES[1]
            ),
        };
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config = parse(&text).with_context(|| format!("in {}", path.display()))?;
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

    /// Where the build command and the local hooks run.
    pub fn build_dir(&self) -> PathBuf {
        self.folder.join(&self.config.build.cwd)
    }

    /// The binary the build produces, if it produces one.
    pub fn binary_path(&self) -> Option<PathBuf> {
        let binary = self.config.build.binary.as_ref()?;
        Some(self.build_dir().join(binary))
    }

    /// The binary's file name, which is also its name in the release.
    pub fn binary_name(&self) -> Option<String> {
        self.config.build.binary.as_deref().map(file_name)
    }

    /// The file whose version names a release: `build.version_from`, else
    /// the first of [`VERSION_FILES`] in `build.cwd`. `None` falls back to
    /// the latest git tag.
    pub fn version_file(&self) -> Option<PathBuf> {
        if let Some(path) = &self.config.build.version_from {
            return Some(self.folder.join(path));
        }
        let dir = self.build_dir();
        VERSION_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
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

    /// The program the service starts, and the rest of `remote.start`.
    pub fn start(&self) -> (Program, String) {
        let (program, rest) = match &self.config.remote.start {
            Some(start) => {
                let start = start.trim();
                match start.split_once(char::is_whitespace) {
                    Some((program, rest)) => (program.to_string(), rest.trim().to_string()),
                    None => (start.to_string(), String::new()),
                }
            }
            None => (
                format!("./{}", self.binary_name().unwrap_or_default()),
                String::new(),
            ),
        };
        let program = if program.starts_with('/') {
            Program::Absolute(program)
        } else if program.contains('/') {
            Program::InRelease(bundle_path(&program))
        } else {
            Program::Tool(program)
        };
        (program, rest)
    }

    /// Programs preflight must find on the server: the start program when
    /// it lives outside the release, then `remote.requires`.
    pub fn required_tools(&self) -> Vec<String> {
        let mut tools = Vec::new();
        if let (Program::Tool(name) | Program::Absolute(name), _) = self.start() {
            tools.push(name);
        }
        for tool in &self.config.remote.requires {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
        tools
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
        check_entries(&self.folder, &self.config.bundle, "bundle entries")
    }

    /// Checks the build left every `build.files` entry where it should be.
    pub fn check_files(&self) -> Result<()> {
        check_entries(
            &self.build_dir(),
            &self.config.build.files,
            "build.files entries",
        )
    }
}

fn check_entries(base: &Path, entries: &[String], what: &str) -> Result<()> {
    let mut problems = Vec::new();
    for entry in entries {
        let path = base.join(entry);
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
        "{} of {} {what} cannot be shipped from {}: {}",
        problems.len(),
        entries.len(),
        base.display(),
        problems.join("; ")
    )
}

/// An entry as it is stored in the release: `./files/` → `files`. Empty for
/// an entry that names the folder itself.
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

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
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

        for (field, hooks) in [
            ("hooks.before", &self.hooks.before),
            ("hooks.server", &self.hooks.server),
            ("hooks.after", &self.hooks.after),
        ] {
            ensure!(
                hooks.iter().all(|hook| !hook.trim().is_empty()),
                "`{field}` has an empty command"
            );
        }

        self.validate_build()?;
        self.validate_remote()?;

        ensure!(
            self.releases.open >= 1,
            "`releases.open` must be at least 1"
        );

        self.validate_contents()?;

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

    fn validate_build(&self) -> Result<()> {
        match &self.build.command {
            Some(CommandLine::Shell(line)) => {
                ensure!(!line.trim().is_empty(), "`build.command` is empty")
            }
            Some(CommandLine::Args(args)) => ensure!(
                args.first().is_some_and(|program| !program.is_empty()),
                "`build.command` is empty"
            ),
            None => {}
        }
        if let Some(binary) = &self.build.binary {
            ensure!(!binary.is_empty(), "`build.binary` is empty");
        }
        ensure!(
            self.build.binary.is_some() || !self.build.files.is_empty(),
            "nothing to ship: set `build.binary` (a compiled program), \
             `build.files` (build output such as dist), or both"
        );
        Ok(())
    }

    fn validate_remote(&self) -> Result<()> {
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

        match &self.remote.start {
            None => ensure!(
                self.build.binary.is_some(),
                "`remote.start` is needed when there is no `build.binary`, \
                 e.g. start: node dist/main.js"
            ),
            Some(start) => {
                ensure!(!start.trim().is_empty(), "`remote.start` is empty");
                ensure!(
                    !start.contains(['\n', '\r', '\\']),
                    "`remote.start` must be one line without backslashes"
                );
                let program = start.split_whitespace().next().unwrap_or_default();
                ensure!(
                    is_program(program),
                    "`remote.start` must begin with a program name or path made of \
                     letters, digits, `.`, `_`, `+`, `-` and `/`, not {program:?}"
                );
            }
        }
        for arg in &self.remote.args {
            ensure!(
                !arg.contains(['"', '\\', '\n', '\r']),
                "`remote.args` entries must not contain quotes, backslashes or newlines: {arg:?}"
            );
        }
        for tool in &self.remote.requires {
            ensure!(
                is_program(tool),
                "`remote.requires` entry {tool:?} must be a program name such as node, \
                 or an absolute path"
            );
        }
        if let Some(user) = &self.remote.run_as {
            ensure!(
                is_plain_name(user),
                "`remote.run_as` may only use letters, digits, `.`, `_` and `-`"
            );
        }
        if let Some(file) = &self.remote.env_file {
            ensure!(
                is_program(file) && !file.starts_with('/'),
                "`remote.env_file` must be a path inside the release, such as .env"
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
        Ok(())
    }

    /// Every `build.files` and `bundle` entry names something inside its
    /// folder, once, without clashing with another entry or with the files
    /// every release carries. Both land at the release root.
    fn validate_contents(&self) -> Result<()> {
        let unit = format!("{}.service", self.remote.service);
        let mut reserved = vec![MANIFEST_FILE.to_string(), unit];
        if let Some(binary) = &self.build.binary {
            reserved.push(file_name(binary));
        }

        // (path in the release, field, entry as written)
        let mut seen: Vec<(String, &str, &str)> = Vec::new();
        for (field, entries, inside) in [
            ("build.files", &self.build.files, "build.cwd"),
            ("bundle", &self.bundle, "the deployable folder"),
        ] {
            for entry in entries {
                let path = Path::new(entry);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    bail!("`{field}` entry {entry:?} must be a path inside {inside}");
                }
                let stored = bundle_path(entry);
                if stored.is_empty() {
                    match field {
                        "bundle" => bail!(
                            "`bundle` entry {entry:?} names the whole deployable folder, \
                             which would ship deploy.yaml too; list the files instead"
                        ),
                        _ => bail!(
                            "`{field}` entry {entry:?} names the whole build folder; \
                             list what to ship instead (dist, package.json, ...)"
                        ),
                    }
                }
                if reserved.contains(&stored) {
                    bail!("`{field}` entry {entry:?} would replace the release's own {stored}");
                }
                for (other, other_field, original) in &seen {
                    if *other == stored {
                        if *other_field == field {
                            bail!("`{field}` lists {stored:?} twice ({original:?} and {entry:?})");
                        }
                        bail!(
                            "{stored:?} would be shipped twice: `{other_field}` {original:?} \
                             and `{field}` {entry:?}"
                        );
                    }
                    let (outer, inner) = if stored.len() < other.len() {
                        (&stored, other)
                    } else {
                        (other, &stored)
                    };
                    if inner.starts_with(&format!("{outer}/")) {
                        bail!(
                            "`{other_field}` {original:?} and `{field}` {entry:?} overlap; \
                             {outer:?} already includes {inner:?}"
                        );
                    }
                }
                seen.push((stored, field, entry));
            }
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

/// `node`, `/usr/bin/node` or `bin/server`: safe in a shell word and a unit
/// line, and never climbing out with `..`.
fn is_program(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-/".contains(c))
        && !text.split('/').any(|part| part == "..")
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
