//! `deploy.yaml`: what one deployable is, where it runs, and the pipeline
//! that ships it.
//!
//! Reading one goes in three passes, so every problem in a file is reported
//! at once with its line: `yaml` keeps where each key was written, `check`
//! walks it against the JSON schema these types generate, and `rules`
//! checks the values. Only a clean file becomes a [`DeployConfig`].

mod check;
mod discover;
pub mod glob;
mod problem;
mod rules;
pub mod schema;
mod yaml;

pub use discover::{Discovery, discover};
pub use problem::{FileProblems, Problem, Span, render};

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;

use crate::ssh::Endpoint;

/// The files that mark a folder as a deployable; a folder may have one of them.
pub const FILE_NAMES: [&str; 2] = ["deploy.yaml", "deploy.yml"];
/// What shipdeck 0.1 read. Found alone, it is reported instead of skipped.
pub const LEGACY_FILE_NAME: &str = "deploy.json";
/// Optional file next to `deploy.yaml` that replaces the built-in systemd unit.
pub const UNIT_TEMPLATE: &str = "service.template";

/// One deployable: what it is, where it runs, and the pipeline that ships it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeployConfig {
    /// The name shipdeck shows, such as `My API (prod)`. Also the systemd
    /// unit's description.
    pub name: String,
    /// Your source folder, relative to this deploy.yaml. Local commands run
    /// here, and `pipeline.pack` takes project files from here. Default: the
    /// folder of this deploy.yaml.
    #[serde(default = "default_project")]
    pub project: String,
    /// How to log in to the server.
    pub ssh: SshTarget,
    /// The systemd service on the server, and where its releases live.
    pub service: Service,
    /// The steps of a deploy, in order: preflight, build, pack, setup,
    /// health, done, and failed when a step fails. upload, unpack, switch and
    /// restart are done by shipdeck and take no settings.
    #[serde(default)]
    pub pipeline: Pipeline,
}

/// How to log in. `host` may be an alias from `~/.ssh/config`; `port` and
/// `user` then come from it unless set here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SshTarget {
    /// The server's address, or an alias from `~/.ssh/config`.
    pub host: String,
    /// Default: the alias's Port, else 22.
    #[serde(default)]
    pub port: Option<u16>,
    /// The user shipdeck logs in and deploys as. Default: the alias's User.
    #[serde(default)]
    pub user: Option<String>,
}

/// The systemd service shipdeck installs, and where its releases live.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// The unit's name without `.service`: `my-api` becomes my-api.service.
    pub name: String,
    /// An absolute folder on the server, such as `/opt/my-api`. It holds
    /// `current` (the live release), `releases/`, `backups/` and `shared/`.
    pub dir: String,
    /// The command the service runs, from the release folder:
    /// `./my-api -E=.env` or `node dist/main.js`. A bare program name such as
    /// `node` is looked up on the server. Default: `./<binary>` from
    /// `pipeline.pack.binary`.
    #[serde(default)]
    pub start: Option<String>,
    /// The user the service runs as. Default: the SSH user.
    #[serde(default)]
    pub run_as: Option<String>,
    /// A `KEY=value` file in the release, such as `.env`. The service gets it
    /// as its environment, and server commands load it too.
    #[serde(default)]
    pub env_file: Option<String>,
    /// Environment variables for the service and for server commands.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Folders kept across releases, such as `uploads`. Each lives in
    /// `shared/` and is linked into every release.
    #[serde(default)]
    pub shared: Vec<String>,
    /// How many old releases stay on the server, for rollback.
    #[serde(default)]
    pub keep: Keep,
}

/// Old releases kept on the server, for rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Keep {
    /// Releases kept unpacked and ready to switch to, counting the live one.
    /// Default: 5.
    #[serde(default = "default_open")]
    pub open: usize,
    /// Older releases kept as `.tar.gz` in `backups/`. Default: 15.
    #[serde(default = "default_compressed")]
    pub compressed: usize,
}

impl Default for Keep {
    fn default() -> Self {
        Self {
            open: default_open(),
            compressed: default_compressed(),
        }
    }
}

/// The steps of a deploy that take settings.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    /// Step 1. shipdeck checks your PC and the server, then runs
    /// `run.server` in `service.dir`, then `run.local` in `project`. A failure
    /// stops the deploy before anything is built.
    #[serde(default)]
    pub preflight: Preflight,
    /// Step 2. Runs `run.local` in `project`. A failure stops the deploy.
    #[serde(default)]
    pub build: Build,
    /// Step 3. What goes into the release: the binary and the files.
    #[serde(default)]
    pub pack: Pack,
    /// Step 6, after shipdeck uploads and unpacks the release. Runs in
    /// `releases/<id>` on the server while the old release is still live:
    /// `run.server`, then `check.server`. A failure removes the new release.
    #[serde(default)]
    pub setup: Setup,
    /// Step 9, after shipdeck switches `current` and restarts the service.
    /// Waits until the service is active, `url` answers and every check
    /// passes; when it does not, see `rollback`.
    #[serde(default)]
    pub health: Health,
    /// Step 10: the new release is live. shipdeck tidies old releases (see
    /// `service.keep`), then runs `run.server` in `current`, then `run.local`.
    /// A failure is only a warning. A manual rollback runs it too, with
    /// SHIPDECK_ACTION=rollback.
    #[serde(default)]
    pub done: Done,
    /// Runs when any step fails, after a rollback if there was one.
    /// SHIPDECK_STEP names the step that failed.
    #[serde(default)]
    pub failed: Failed,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preflight {
    /// Programs the server must have, such as `node` or `npm`. Preflight
    /// stops if one is missing.
    #[serde(default)]
    pub requires: Vec<String>,
    /// `server` runs first, in `service.dir`; then `local`, in `project`.
    #[serde(default)]
    pub run: LocalAndServer,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// The file whose version names the release, relative to `project`:
    /// Cargo.toml, package.json, pyproject.toml, or any file whose first line
    /// is the version. Default: the first of those in `project`, else the
    /// latest git tag.
    #[serde(default)]
    pub version_from: Option<String>,
    /// Builds the project on your PC, in `project`.
    #[serde(default)]
    pub run: LocalOnly,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    /// A compiled program, relative to `project`. It lands at the release
    /// root and is checked against the server's CPU and glibc.
    #[serde(default)]
    pub binary: Option<String>,
    /// Files and folders to put in the release, each at its own path.
    #[serde(default)]
    pub files: Files,
}

/// Paths or globs, fast-glob and Tailwind style: `*` and `?` inside one
/// folder, `**/` for any number of folders, `[a-z]`, `{js,ts}`. A pattern
/// starting with `!` turns it around. `.git` is never shipped.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Files {
    /// What to ship. A folder brings everything in it; `"!pattern"` leaves
    /// out what it matches.
    #[serde(default)]
    pub include: Sources,
    /// What to leave out of `include`. A folder leaves out everything in it;
    /// `"!pattern"` keeps what it matches.
    #[serde(default)]
    pub exclude: Sources,
}

/// Each path keeps its place inside the release.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    /// Relative to `project`, such as `dist` or `"src/**/*.{js,json}"`. `.`
    /// is the whole project.
    #[serde(default)]
    pub project: Vec<String>,
    /// Relative to this deploy.yaml's folder, for files that belong to this
    /// deployment, such as `.env`. A file here replaces a project file at the
    /// same path.
    #[serde(default)]
    pub deploy: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    /// Commands in the new release on the server, such as
    /// `npm ci --omit=dev` or migrations.
    #[serde(default)]
    pub run: ServerOnly,
    /// Commands that must pass after `run`, such as
    /// `node --check dist/main.js`.
    #[serde(default)]
    pub check: ServerOnly,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// Fetched on the server with curl or wget; it must answer 2xx. Such as
    /// `http://127.0.0.1:3000/health`.
    #[serde(default)]
    pub url: Option<String>,
    /// Commands that must pass: `server` in `current`, then `local` in
    /// `project`. Retried until `timeout_secs`.
    #[serde(default)]
    pub check: LocalAndServer,
    /// Seconds the release has to become healthy. Default: 20.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// What happens when the new release is not healthy, or the switch or
    /// restart fails. Default: ask.
    #[serde(default)]
    pub rollback: Rollback,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            url: None,
            check: LocalAndServer::default(),
            timeout_secs: default_timeout(),
            rollback: Rollback::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Rollback {
    /// Ask before rolling back to the release that was live. With no answer
    /// within 30 seconds, roll back.
    #[default]
    Ask,
    /// Roll back at once.
    Auto,
}

/// How long the rollback question waits before rolling back on its own.
pub const ROLLBACK_ANSWER_SECS: u64 = 30;

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Done {
    /// `server` runs first, in `current`; then `local`, in `project`.
    #[serde(default)]
    pub run: LocalAndServer,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Failed {
    /// Runs on your PC, in `project`. The server may be out of reach when a
    /// step fails, so there is no `server` here.
    #[serde(default)]
    pub run: LocalOnly,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalAndServer {
    /// On your PC, in `project`.
    #[serde(default)]
    pub local: Commands,
    /// On the server.
    #[serde(default)]
    pub server: Commands,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalOnly {
    /// On your PC, in `project`.
    #[serde(default)]
    pub local: Commands,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServerOnly {
    /// On the server.
    #[serde(default)]
    pub server: Commands,
}

/// Shell commands run with `sh -c`, so `&&`, pipes and `$VARS` work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commands(pub Vec<String>);

impl Commands {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

impl<'de> Deserialize<'de> for Commands {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OneOrMany {
            One(String),
            Many(Vec<String>),
        }
        Ok(Self(match OneOrMany::deserialize(deserializer)? {
            OneOrMany::One(command) => vec![command],
            OneOrMany::Many(commands) => commands,
        }))
    }
}

impl JsonSchema for Commands {
    fn schema_name() -> Cow<'static, str> {
        "Commands".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "One shell command, or a list run in order that stops at the first failure.",
            "anyOf": [
                { "type": "string" },
                { "type": "array", "items": { "type": "string" } }
            ]
        })
    }
}

fn default_project() -> String {
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

/// Files whose version names a release, looked for in `project` in this
/// order when `pipeline.build.version_from` is not set.
pub const VERSION_FILES: [&str; 4] = ["Cargo.toml", "package.json", "pyproject.toml", "VERSION"];

/// Reads the text of a `deploy.yaml`, or says everything wrong with it.
pub fn parse(text: &str) -> Result<DeployConfig, Vec<Problem>> {
    parse_tree(text).map(|(config, _)| config)
}

/// Like [`parse`], also handing back the tree so later problems can point
/// at a line.
fn parse_tree(text: &str) -> Result<(DeployConfig, yaml::Node), Vec<Problem>> {
    let root = yaml::parse(text).map_err(|problem| vec![problem])?;
    let checked = check::structure(&root);
    let mut problems = checked.problems;
    // What fits is still read, so value problems show up in the same run.
    match serde_json::from_value::<DeployConfig>(checked.value) {
        Ok(_) if checked.old_format => {}
        Ok(config) => {
            problems.extend(rules::check(&config, &root));
            if problems.is_empty() {
                return Ok((config, root));
            }
        }
        Err(err) if problems.is_empty() => {
            problems.push(Problem::new(format!(
                "deploy.yaml could not be read: {err}"
            )));
        }
        Err(_) => {}
    }
    problems.sort_by_key(|p| p.at.map(|at| at.start));
    Err(problems)
}

/// The config file in `folder`, if it has one.
pub fn config_file(folder: &Path) -> Option<PathBuf> {
    FILE_NAMES
        .iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
}

/// Where the program in `service.start` (or the binary) comes from.
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
    /// Reads `folder`'s config, or every problem with it.
    pub fn load(folder: &Path) -> Result<Self, FileProblems> {
        let found: Vec<PathBuf> = FILE_NAMES
            .iter()
            .map(|name| folder.join(name))
            .filter(|path| path.is_file())
            .collect();
        let path = match found.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(FileProblems::without_source(
                    folder.join(FILE_NAMES[0]),
                    Problem::new(format!("{} has no {}", folder.display(), FILE_NAMES[0])),
                ));
            }
            _ => {
                return Err(FileProblems::without_source(
                    found[0].clone(),
                    Problem::new(format!(
                        "this folder has both {} and {}",
                        FILE_NAMES[0], FILE_NAMES[1]
                    ))
                    .help("keep one of them"),
                ));
            }
        };
        let text = std::fs::read_to_string(&path).map_err(|err| {
            FileProblems::without_source(
                path.clone(),
                Problem::new(format!("cannot read it: {err}")),
            )
        })?;
        let failed = |problems| FileProblems {
            path: path.clone(),
            source: Some(text.clone()),
            problems,
        };
        let (config, root) = parse_tree(&text).map_err(failed)?;
        let endpoint = Endpoint::resolve(&config.ssh).map_err(|err| {
            failed(vec![
                Problem::new(format!("{err:#}"))
                    .at(root
                        .span_of(&["ssh", "user"])
                        .or_else(|| root.key_span(&["ssh"])))
                    .label("cannot log in like this"),
            ])
        })?;
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

    /// Where local commands run and project files come from.
    pub fn project_dir(&self) -> PathBuf {
        self.folder.join(&self.config.project)
    }

    /// The binary the build produces, if it produces one.
    pub fn binary_path(&self) -> Option<PathBuf> {
        let binary = self.config.pipeline.pack.binary.as_ref()?;
        Some(self.project_dir().join(binary))
    }

    /// The binary's file name, which is also its name in the release.
    pub fn binary_name(&self) -> Option<String> {
        self.config.pipeline.pack.binary.as_deref().map(file_name)
    }

    /// The file whose version names a release: `build.version_from`, else
    /// the first of [`VERSION_FILES`] in `project`. `None` falls back to the
    /// latest git tag.
    pub fn version_file(&self) -> Option<PathBuf> {
        let dir = self.project_dir();
        if let Some(path) = &self.config.pipeline.build.version_from {
            return Some(dir.join(path));
        }
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
            .service
            .run_as
            .as_deref()
            .unwrap_or(&self.endpoint.user)
    }

    /// The program the service starts, and the rest of `service.start`.
    pub fn start(&self) -> (Program, String) {
        let (program, rest) = match &self.config.service.start {
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
            Program::InRelease(release_path(&program))
        } else {
            Program::Tool(program)
        };
        (program, rest)
    }

    /// Programs preflight must find on the server: the start program when
    /// it lives outside the release, then `preflight.requires`.
    pub fn required_tools(&self) -> Vec<String> {
        let mut tools = Vec::new();
        if let (Program::Tool(name) | Program::Absolute(name), _) = self.start() {
            tools.push(name);
        }
        for tool in &self.config.pipeline.preflight.requires {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
        tools
    }
}

/// A path as it is stored in the release: `./files/` → `files`. Empty for a
/// path that names the folder itself.
pub fn release_path(entry: &str) -> String {
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

/// `node`, `/usr/bin/node` or `bin/server`: safe in a shell word and a unit
/// line, and never climbing out with `..`.
fn is_program(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-/".contains(c))
        && !text.split('/').any(|part| part == "..")
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
