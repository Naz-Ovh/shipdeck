//! The root file and the deploy file as written, before paths are resolved.

use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use super::steps::{AppSteps, ReleaseSteps};

/// The root file: its deploys, by name, each a deploy file's path or a deploy
/// written in place.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RootFile {
    /// Marks the file as a root file. A deploy file never holds it.
    #[schemars(schema_with = "root_true")]
    pub root: bool,
    /// Deploy names, and the deploy file of each or the deploy itself.
    pub deploys: BTreeMap<String, DeployEntry>,
}

/// The schema of `root`: the constant `true`.
fn root_true(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "description": "Marks the file as a root file; a deploy file never holds it.",
        "const": true
    })
}

/// One entry of a root file: a deploy file's path, or the deploy in place.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum DeployEntry {
    /// A deploy file's path on your PC.
    File(String),
    /// The deploy written in the root file.
    InPlace(Box<DeployFile>),
}

/// One whole deploy: the server, the release and every app.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeployFile {
    /// The deploy's name; the default process name starts with it.
    pub name: String,
    /// The server it runs on.
    pub server: Server,
    /// Settings for the whole release; each app adds its own.
    #[serde(default)]
    pub release: Release,
    /// The apps of the deploy, by name. They run in name order.
    pub apps: BTreeMap<String, App>,
}

/// Where the deploy runs.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// How to log in.
    pub ssh: SshTarget,
    /// The server folder that holds `current`, `releases/`, `backups/` and
    /// `shared/`. Absolute, such as `/opt/shop`.
    pub dir: String,
}

/// How to log in. `host` may be an alias from `~/.ssh/config`; `port` and
/// `user` then come from it unless set here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
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
    /// Give server commands the environment a login shell has: PATH from
    /// nvm and the like. Default: true.
    #[serde(default = "default_true")]
    pub shell_env: bool,
}

/// The whole release: files that belong to no app, and the release commands.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// Files shipped beside the apps, each at its path from the project folder.
    #[serde(default)]
    pub files: Vec<FileEntry>,
    /// Patterns left out of `files`, matched against the path from the
    /// project folder.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Folders kept across releases, each at its path in the release.
    #[serde(default)]
    pub shared: Vec<String>,
    /// The release commands and settings of every step.
    #[serde(default)]
    pub steps: ReleaseSteps,
}

/// One app: a process to run, or a static site to serve.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct App {
    /// The app's source folder on your PC. Its files keep their path inside
    /// it, and its local commands run there. Default: the project folder.
    #[serde(default)]
    pub source: Option<String>,
    /// Files shipped for this app.
    #[serde(default)]
    pub files: Vec<FileEntry>,
    /// Patterns left out of `files`, matched against the path from the
    /// project folder.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// A compiled program: a path, or a path and its place in the app folder.
    /// It lands with mode 0755 and is the default `process.start`.
    #[serde(default)]
    pub binary: Option<FileEntry>,
    /// Folders kept across releases, in this app's folder.
    #[serde(default)]
    pub shared: Vec<String>,
    /// A running program. Use this or `static: true`, not both.
    #[serde(default)]
    pub process: Option<Process>,
    /// Serve the files instead of running a process. Must be `true`.
    #[serde(default, rename = "static")]
    pub static_site: Option<bool>,
    /// nginx settings for this app. Without them shipdeck does not touch
    /// nginx.
    #[serde(default)]
    pub nginx: Option<Nginx>,
    /// This app's commands and settings of every step.
    #[serde(default)]
    pub steps: AppSteps,
}

/// A running program and the manager that keeps it up.
#[derive(Debug, Clone)]
pub struct Process {
    /// The process name. Default: `<name>-<app>`.
    pub name: Option<String>,
    /// The command that runs the app, in the app folder. Default: `./<binary>`.
    pub start: Option<String>,
    /// Environment variables for the app and its server commands.
    pub env: BTreeMap<String, String>,
    /// A `KEY=value` file in the app folder, such as `.env`.
    pub env_file: Option<String>,
    /// The manager that keeps the app up, and its settings. Exactly one.
    pub manager: Option<KeepUp>,
}

/// The one manager a process has, and its settings.
#[derive(Debug, Clone)]
pub enum KeepUp {
    /// A systemd unit.
    Systemd(Systemd),
    /// A PM2 app.
    Pm2(Pm2),
    /// An oxmgr app. Parsed and checked; a deploy stops at preflight.
    Oxmgr(Oxmgr),
}

/// A running program and the manager that keeps it up.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawProcess {
    /// The process name. Default: `<name>-<app>`.
    #[serde(default)]
    name: Option<String>,
    /// The command that runs the app, in the app folder. Default: `./<binary>`.
    #[serde(default)]
    start: Option<String>,
    /// Environment variables for the app and its server commands.
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// A `KEY=value` file in the app folder, such as `.env`.
    #[serde(default)]
    env_file: Option<String>,
    /// A systemd unit. Use exactly one of `systemd`, `pm2` or `oxmgr`.
    #[serde(default)]
    systemd: Option<Systemd>,
    /// A PM2 app. Use exactly one of `systemd`, `pm2` or `oxmgr`.
    #[serde(default)]
    pm2: Option<Pm2>,
    /// An oxmgr app. Use exactly one of `systemd`, `pm2` or `oxmgr`.
    #[serde(default)]
    oxmgr: Option<Oxmgr>,
}

impl Process {
    /// The process as the file writes it.
    fn raw(&self) -> RawProcess {
        let (systemd, pm2, oxmgr) = match &self.manager {
            Some(KeepUp::Systemd(settings)) => (Some(settings.clone()), None, None),
            Some(KeepUp::Pm2(settings)) => (None, Some(settings.clone()), None),
            Some(KeepUp::Oxmgr(settings)) => (None, None, Some(settings.clone())),
            None => (None, None, None),
        };
        RawProcess {
            name: self.name.clone(),
            start: self.start.clone(),
            env: self.env.clone(),
            env_file: self.env_file.clone(),
            systemd,
            pm2,
            oxmgr,
        }
    }
}

impl Serialize for Process {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Process {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawProcess::deserialize(deserializer)?;
        let mut chosen: Vec<KeepUp> = Vec::new();
        if let Some(settings) = raw.systemd {
            chosen.push(KeepUp::Systemd(settings));
        }
        if let Some(settings) = raw.pm2 {
            chosen.push(KeepUp::Pm2(settings));
        }
        if let Some(settings) = raw.oxmgr {
            chosen.push(KeepUp::Oxmgr(settings));
        }
        if chosen.len() > 1 {
            return Err(de::Error::custom("a process takes exactly one manager"));
        }
        Ok(Process {
            name: raw.name,
            start: raw.start,
            env: raw.env,
            env_file: raw.env_file,
            manager: chosen.pop(),
        })
    }
}

impl JsonSchema for Process {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Process".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        RawProcess::json_schema(generator)
    }
}

/// A systemd unit: the built-in one with your own properties, or your own file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Systemd {
    /// The user the unit runs as. Default: the SSH user.
    #[serde(default)]
    pub user: Option<String>,
    /// Unit properties by section, in systemd's own spelling. A property
    /// replaces the built-in unit's line of that name, or is added.
    #[serde(default)]
    pub options: Option<SystemdOptions>,
    /// A whole unit file on your PC. It replaces `options`.
    #[serde(default)]
    pub template: Option<String>,
    /// Planned, not supported yet. An error when set.
    #[serde(default)]
    pub instances: Option<u32>,
}

/// The systemd unit sections `options` writes into.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemdOptions {
    /// `[Unit]` properties.
    #[serde(default, rename = "Unit")]
    pub unit: BTreeMap<String, serde_json::Value>,
    /// `[Service]` properties.
    #[serde(default, rename = "Service")]
    pub service: BTreeMap<String, serde_json::Value>,
    /// `[Install]` properties.
    #[serde(default, rename = "Install")]
    pub install: BTreeMap<String, serde_json::Value>,
}

/// A PM2 app: the file shipdeck writes, or your own ecosystem file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pm2 {
    /// PM2 app properties, such as `instances` or `max_memory_restart`.
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
    /// Your own PM2 file, in the app folder. It replaces `options`.
    #[serde(default)]
    pub ecosystem: Option<String>,
}

/// An oxmgr app. Parsed and checked; not built yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Oxmgr {
    /// oxfile app properties.
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
}

/// nginx settings for one app. Parsed and checked; not built yet.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Nginx {
    /// The domains this app answers on.
    #[serde(default)]
    pub domains: Vec<String>,
    /// A certificate for those domains, or `none` for plain HTTP.
    #[serde(default)]
    pub tls: Option<Tls>,
    /// Where the app listens, for a process app.
    #[serde(default)]
    pub proxy_pass: Option<String>,
    /// The folder nginx serves, in the app folder, for a static app.
    #[serde(default)]
    pub root: Option<String>,
    /// The file nginx serves for a folder, for a static app.
    #[serde(default)]
    pub index: Option<String>,
    /// Response headers to add.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Extra `location` blocks, by path.
    #[serde(default)]
    pub locations: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// Raw nginx directives, added to the server block as they are.
    #[serde(default)]
    pub raw: Option<String>,
}

pub use super::commands::{Command, CommandList, Runner};
pub use super::steps::{
    AppBackupStep, AppHealthStep, BackupStep, BuildStep, Commands, HealthStep, Keep, PreflightStep,
    RollbackMode, RollbackStep, Step,
};
pub use super::values::{FileEntry, MappedFile, Tls, TlsNone, TlsProvider};

fn default_true() -> bool {
    true
}
