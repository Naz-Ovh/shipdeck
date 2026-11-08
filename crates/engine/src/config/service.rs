//! Service and process manager configuration.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Program, release_path};

/// The service shipdeck runs, and where its releases live.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// The unit's name without `.service`: `my-api` becomes my-api.service.
    /// With PM2, also the app's name unless `pm2.app` is set.
    pub name: String,
    /// An absolute folder on the server, such as `/opt/my-api`. It holds
    /// `current` (the live release), `releases/`, `backups/` and `shared/`.
    /// Read from the top-level `dir`; never written inside `service`.
    #[serde(skip)]
    #[schemars(skip)]
    pub dir: String,
    /// What keeps the app running: `systemd`, a unit shipdeck installs with
    /// sudo, or `pm2`, run as the SSH user without sudo. Default: systemd.
    #[serde(default)]
    pub manager: Manager,
    /// How PM2 runs the app, with `manager: pm2`.
    #[serde(default)]
    pub pm2: Pm2,
    /// The command the service runs, from the release folder:
    /// `./my-api -E=.env` or `node dist/main.js`. A bare program name such as
    /// `node` is looked up on the server. Default: `./<binary>` from
    /// `pipeline.pack.binary`.
    #[serde(default)]
    pub start: Option<String>,
    /// The user the service runs as. Default: the SSH user. PM2 always runs
    /// the app as the SSH user.
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
    /// The folder a static site serves, when this is not a process.
    /// Read from `static.root`; never written in `service:`.
    #[serde(skip)]
    #[schemars(skip)]
    pub static_root: Option<String>,
}

impl Service {
    /// A static site: files in a release, with no process to keep up.
    #[must_use]
    pub fn static_site(
        name: impl Into<String>,
        dir: impl Into<String>,
        root: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            dir: dir.into(),
            manager: Manager::Static,
            pm2: Pm2::default(),
            start: None,
            run_as: None,
            env_file: None,
            env: BTreeMap::new(),
            shared: Vec::new(),
            static_root: Some(root.into()),
        }
    }

    /// Whether this deployable serves files instead of running a process.
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.static_root.is_some()
    }

    /// The folder inside the release that nginx serves, for a static site.
    #[must_use]
    pub fn static_root(&self) -> Option<&str> {
        self.static_root.as_deref()
    }

    /// The live folder a web server reads, after the switch.
    #[must_use]
    pub fn served_path(&self, release_dir: &str) -> Option<String> {
        let root = release_path(self.static_root()?);
        let app = if release_dir.is_empty() {
            String::new()
        } else {
            format!("{release_dir}/")
        };
        Some(format!("{}/current/{app}{root}", self.dir))
    }

    /// The start program and arguments, using the packed binary as a default.
    #[must_use]
    pub fn command(&self, binary: Option<&str>) -> (Program, String) {
        let (program, rest) = match &self.start {
            Some(start) => {
                let start = start.trim();
                match start.split_once(char::is_whitespace) {
                    Some((program, rest)) => (program.to_string(), rest.trim().to_string()),
                    None => (start.to_string(), String::new()),
                }
            }
            None => (format!("./{}", binary.unwrap_or_default()), String::new()),
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

    /// The file shipdeck writes into each release for the manager:
    /// `<name>.service`, or `<name>.pm2.json` when there is no
    /// `pm2.ecosystem`. A static site has no process file.
    #[must_use]
    pub fn own_file(&self) -> Option<String> {
        match self.manager {
            Manager::Systemd => Some(format!("{}.service", self.name)),
            Manager::Pm2 if self.pm2.ecosystem.is_some() => None,
            Manager::Pm2 => Some(format!("{}.pm2.json", self.name)),
            Manager::Static => None,
        }
    }

    /// The PM2 app's name.
    #[must_use]
    pub fn pm2_app(&self) -> &str {
        self.pm2.app.as_deref().unwrap_or(&self.name)
    }

    /// The file in the release PM2 starts the app from.
    #[must_use]
    pub fn pm2_file(&self) -> String {
        match &self.pm2.ecosystem {
            Some(file) => release_path(file),
            None => format!("{}.pm2.json", self.name),
        }
    }
}

/// What keeps the app running on the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Manager {
    /// A systemd unit, installed and restarted with sudo.
    #[default]
    Systemd,
    /// PM2, run as the SSH user: no sudo. Reloaded with
    /// `pm2 startOrReload`, then saved with `pm2 save`.
    Pm2,
    /// No process: a web server serves the release's files.
    #[serde(skip)]
    #[schemars(skip)]
    Static,
}

impl Manager {
    /// As messages name it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
            Self::Pm2 => "PM2",
            Self::Static => "static",
        }
    }
}

/// How PM2 runs the app. Either shipdeck writes the app file from
/// `service.start`, `service.env` and `options`, or `ecosystem` names your own.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pm2 {
    /// The app's name in PM2, as `pm2 list` shows it. Default: `service.name`.
    #[serde(default)]
    pub app: Option<String>,
    /// Your own PM2 file in the release, such as `ecosystem.config.js`.
    /// shipdeck reloads the app `app` from it; `service.start` and `options`
    /// are not used. Ship it with `pipeline.pack.files`.
    #[serde(default)]
    pub ecosystem: Option<String>,
    /// More PM2 app settings for the file shipdeck writes, such as
    /// `instances: max`, `exec_mode: cluster` or `max_memory_restart: 300M`.
    /// shipdeck sets `name`, `cwd` and the environment itself.
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
}
