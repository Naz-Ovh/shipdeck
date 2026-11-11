//! The resolved deploy: the deploy file with its local folders and endpoint,
//! used by everything after loading.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::names::{file_name, split_args};
use super::paths::{self, Form};
use super::placeholders::{self, AppValues, Values};
use super::types::{
    App, AppHealthStep, DeployFile, FileEntry, HealthStep, Keep, KeepUp, Oxmgr, Pm2, Process,
    RollbackMode, Systemd,
};
use super::{Problem, ReleaseSteps};
use crate::ssh::Endpoint;

/// Files whose version names a release, looked for in the project folder
/// when `release.steps.build.version_from` is not set.
pub const VERSION_FILES: [&str; 4] = ["Cargo.toml", "package.json", "pyproject.toml", "VERSION"];

/// What keeps a process running, and its settings.
///
/// The pipeline sees one value per app, so two managers cannot be carried
/// past the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manager {
    /// A systemd unit.
    Systemd(Systemd),
    /// A PM2 app.
    Pm2(Pm2),
    /// An oxmgr app. Parsed and checked; a deploy stops at preflight.
    Oxmgr(Oxmgr),
}

impl Manager {
    /// As the screens and messages name it.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Systemd(_) => "systemd",
            Self::Pm2(_) => "PM2",
            Self::Oxmgr(_) => "oxmgr",
        }
    }

    /// The manager as a value with no settings, for `init` and for matching.
    #[must_use]
    pub fn kind(&self) -> ManagerKind {
        match self {
            Self::Systemd(_) => ManagerKind::Systemd,
            Self::Pm2(_) => ManagerKind::Pm2,
            Self::Oxmgr(_) => ManagerKind::Oxmgr,
        }
    }
}

/// Which manager it is, without its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManagerKind {
    /// A systemd unit.
    Systemd,
    /// A PM2 app.
    Pm2,
    /// An oxmgr app.
    Oxmgr,
}

/// Where the program in `process.start` comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// `node`: looked up on the server's `PATH` at preflight.
    Tool(String),
    /// `/usr/bin/node`: used as written.
    Absolute(String),
    /// `./my-api`, `bin/server`: a file inside the app folder.
    InRelease(String),
}

/// One deploy, with its paths resolved on this machine.
#[derive(Debug, Clone)]
pub struct Deployment {
    /// The deploy's handle: its key in the root file, or its `name`.
    pub slug: String,
    /// The deploy file.
    pub file: PathBuf,
    /// The folder shipdeck runs in.
    pub project: PathBuf,
    /// The deploy file's folder.
    pub config_dir: PathBuf,
    /// The deploy file's text.
    pub yaml: String,
    /// The connection, with `~/.ssh/config` applied.
    pub endpoint: Endpoint,
    /// Server details that did not come from the file, such as
    /// `host from --host`.
    pub ssh_from: Vec<String>,
    /// Why the deploy cannot run: a variable it names is not set. It is
    /// listed, and nothing runs for it.
    pub unavailable: Option<String>,
    /// The deploy file as written, with defaults applied.
    pub deploy: DeployFile,
}

impl Deployment {
    /// The deploy's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.deploy.name
    }

    /// The server folder.
    #[must_use]
    pub fn dir(&self) -> &str {
        &self.deploy.server.dir
    }

    /// The apps in name order.
    pub fn apps(&self) -> impl Iterator<Item = (&String, &App)> {
        self.deploy.apps.iter()
    }

    /// The names of the apps in name order.
    #[must_use]
    pub fn app_names(&self) -> Vec<&str> {
        self.deploy.apps.keys().map(String::as_str).collect()
    }

    /// The first app that runs a process, in name order.
    #[must_use]
    pub fn first_process(&self) -> Option<&str> {
        self.deploy
            .apps
            .iter()
            .find(|(_, app)| app.process.is_some())
            .map(|(name, _)| name.as_str())
    }

    /// Whether every app serves files and none runs.
    #[must_use]
    pub fn has_process(&self) -> bool {
        self.deploy.apps.values().any(|app| app.process.is_some())
    }

    /// The release settings for the whole deploy.
    #[must_use]
    pub fn release(&self) -> &super::Release {
        &self.deploy.release
    }

    /// The release steps.
    #[must_use]
    pub fn steps(&self) -> &ReleaseSteps {
        &self.deploy.release.steps
    }

    /// One app by name.
    #[must_use]
    pub fn app(&self, name: &str) -> Option<&App> {
        self.deploy.apps.get(name)
    }

    /// The app's process name: `process.name`, or `<name>-<app>`.
    #[must_use]
    pub fn process_name(&self, app: &str) -> String {
        self.app(app)
            .and_then(|app| app.process.as_ref())
            .and_then(|process| process.name.clone())
            .unwrap_or_else(|| format!("{}-{app}", self.deploy.name))
    }

    /// The app's process, when it has one.
    #[must_use]
    pub fn process(&self, app: &str) -> Option<&Process> {
        self.app(app)?.process.as_ref()
    }

    /// The manager that keeps the app up, when it has a process.
    #[must_use]
    pub fn manager(&self, app: &str) -> Option<Manager> {
        Some(match self.process(app)?.manager.as_ref()? {
            KeepUp::Systemd(settings) => Manager::Systemd(settings.clone()),
            KeepUp::Pm2(settings) => Manager::Pm2(settings.clone()),
            KeepUp::Oxmgr(settings) => Manager::Oxmgr(settings.clone()),
        })
    }

    /// The app's systemd settings, when it has them.
    #[must_use]
    pub fn systemd(&self, app: &str) -> Option<&Systemd> {
        match self.process(app)?.manager.as_ref()? {
            KeepUp::Systemd(settings) => Some(settings),
            KeepUp::Pm2(_) | KeepUp::Oxmgr(_) => None,
        }
    }

    /// The app's PM2 settings, when it has them.
    #[must_use]
    pub fn pm2(&self, app: &str) -> Option<&Pm2> {
        match self.process(app)?.manager.as_ref()? {
            KeepUp::Pm2(settings) => Some(settings),
            KeepUp::Systemd(_) | KeepUp::Oxmgr(_) => None,
        }
    }

    /// Whether the app serves files and runs nothing.
    #[must_use]
    pub fn is_static(&self, app: &str) -> bool {
        self.app(app)
            .is_some_and(|app| app.static_site == Some(true))
    }

    /// The app's source folder on this machine.
    #[must_use]
    pub fn source(&self, app: &str) -> PathBuf {
        let written = self.source_text(app);
        paths::lexical(&paths::local(&self.project, &self.config_dir, written))
    }

    /// The app's `source` as written.
    #[must_use]
    pub fn source_text(&self, app: &str) -> &str {
        self.app(app)
            .and_then(|app| app.source.as_deref())
            .unwrap_or(".")
    }

    /// The app's folder in the release: `apps/<app>`.
    #[must_use]
    pub fn release_dir(&self, app: &str) -> String {
        format!("apps/{app}")
    }

    /// The app's binary entry, in either form.
    #[must_use]
    pub fn binary(&self, app: &str) -> Option<&FileEntry> {
        self.app(app)?.binary.as_ref()
    }

    /// The app's binary on this machine.
    #[must_use]
    pub fn binary_local(&self, app: &str) -> Option<PathBuf> {
        let binary = self.binary(app)?;
        let written = match binary {
            FileEntry::Path(path) => path,
            FileEntry::To(mapped) => &mapped.path,
        };
        Some(paths::local(&self.project, &self.config_dir, written))
    }

    /// The binary's own file name, which is its place when `to` is absent.
    #[must_use]
    pub fn binary_name(&self, app: &str) -> Option<String> {
        match self.binary(app)? {
            FileEntry::Path(path) => Some(file_name(path)),
            FileEntry::To(mapped) => Some(match &mapped.to {
                Some(to) => file_name(to),
                None => file_name(&mapped.path),
            }),
        }
    }

    /// Where the binary lands inside the app folder: `to`, or its file name.
    #[must_use]
    pub fn binary_place(&self, app: &str) -> Option<String> {
        match self.binary(app)? {
            FileEntry::Path(path) => Some(file_name(path)),
            FileEntry::To(mapped) => match &mapped.to {
                Some(to) => paths::release_path(to).ok(),
                None => Some(file_name(&mapped.path)),
            },
        }
    }

    /// The program the app runs and the rest of its `start`.
    #[must_use]
    pub fn start(&self, app: &str) -> Option<(Program, String)> {
        let process = self.process(app)?;
        let start = match &process.start {
            Some(start) => start.trim().to_string(),
            None => format!("./{}", self.binary_place(app)?),
        };
        let (program, rest) = match start.split_once(char::is_whitespace) {
            Some((program, rest)) => (program.to_string(), rest.trim().to_string()),
            None => (start, String::new()),
        };
        let program = if program.starts_with('/') {
            Program::Absolute(program)
        } else if program.contains('/') {
            Program::InRelease(paths::release_path(&program).unwrap_or(program))
        } else {
            Program::Tool(program)
        };
        Some((program, rest))
    }

    /// The user the app runs as.
    #[must_use]
    pub fn run_user(&self, app: &str) -> String {
        self.systemd(app)
            .and_then(|systemd| systemd.user.clone())
            .unwrap_or_else(|| self.endpoint.user.clone())
    }

    /// The app's environment file, relative to its folder in the release.
    #[must_use]
    pub fn env_file(&self, app: &str) -> Option<String> {
        self.process(app)?.env_file.clone()
    }

    /// The app's PM2 ecosystem file, relative to its folder in the release.
    #[must_use]
    pub fn pm2_ecosystem(&self, app: &str) -> Option<String> {
        self.pm2(app)?.ecosystem.clone()
    }

    /// The app's PM2 file in the release: `.shipdeck/<process>.pm2.json`
    /// unless the app carries its own ecosystem file.
    #[must_use]
    pub fn pm2_file(&self, app: &str) -> String {
        match self.pm2_ecosystem(app) {
            Some(file) => paths::release_path(&file)
                .map_or_else(|_| file, |file| paths::join(&self.release_dir(app), &file)),
            None => format!(".shipdeck/{}.pm2.json", self.process_name(app)),
        }
    }

    /// The app's systemd unit file in the release.
    #[must_use]
    pub fn unit_file(&self, app: &str) -> String {
        format!(".shipdeck/{}.service", self.process_name(app))
    }

    /// The app's systemd template on this machine, when it has one.
    #[must_use]
    pub fn unit_template(&self, app: &str) -> Option<PathBuf> {
        let template = self.systemd(app)?.template.as_deref()?;
        Some(paths::local(&self.project, &self.config_dir, template))
    }

    /// The file whose version names the release: `version_from`, else the
    /// first of [`VERSION_FILES`] in the project folder.
    #[must_use]
    pub fn version_file(&self) -> Option<PathBuf> {
        if let Some(file) = &self.steps().build.version_from {
            return Some(paths::local(&self.project, &self.config_dir, file));
        }
        VERSION_FILES
            .iter()
            .map(|name| self.project.join(name))
            .find(|path| path.is_file())
    }

    /// How many old releases stay.
    #[must_use]
    pub fn keep(&self) -> Keep {
        self.steps().backup.keep.unwrap_or_default()
    }

    /// What an automatic rollback does.
    #[must_use]
    pub fn rollback_mode(&self) -> RollbackMode {
        self.steps().rollback.mode.unwrap_or_default()
    }

    /// How long `ask` waits before rolling back.
    #[must_use]
    pub fn ask_secs(&self) -> u64 {
        self.steps().rollback.ask_secs.unwrap_or(30)
    }

    /// Whether a failed release stays for inspection.
    #[must_use]
    pub fn keep_failed(&self) -> bool {
        self.steps().rollback.keep_failed.unwrap_or(false)
    }

    /// How long the release has to become healthy.
    #[must_use]
    pub fn health_timeout(&self) -> u64 {
        self.steps().health.timeout_secs
    }

    /// The release's health commands.
    #[must_use]
    pub fn release_health(&self) -> &HealthStep {
        &self.steps().health
    }

    /// An app's health commands and URL.
    #[must_use]
    pub fn app_health(&self, app: &str) -> Option<&AppHealthStep> {
        Some(&self.app(app)?.steps.health)
    }

    /// The resolved deploy as `.shipdeck/settings.json` holds it: the deploy
    /// file with its placeholders unresolved.
    ///
    /// # Errors
    /// Returns an error when it cannot be written as JSON.
    pub fn settings(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(&self.deploy)?)
    }

    /// Reads the settings a release carries and resolves them again.
    ///
    /// # Errors
    /// Returns the problem when the settings cannot be read or checked.
    pub fn from_settings(
        bytes: &[u8],
        project: &Path,
        config_dir: &Path,
        file: PathBuf,
        slug: String,
    ) -> Result<Self, Problem> {
        let deploy: DeployFile = serde_json::from_slice(bytes).map_err(|err| {
            Problem::new(format!("this release's settings cannot be read: {err}"))
        })?;
        if !super::rules::valid(&deploy) {
            return Err(Problem::new(
                "this release's settings are not valid config format 2",
            ));
        }
        let endpoint = Endpoint::resolve(&deploy.server.ssh).map_err(|err| {
            Problem::new(format!(
                "this release's server settings cannot be used: {err:#}"
            ))
        })?;
        Ok(Self {
            slug,
            file,
            project: project.to_path_buf(),
            config_dir: config_dir.to_path_buf(),
            yaml: String::new(),
            endpoint,
            ssh_from: Vec::new(),
            unavailable: None,
            deploy,
        })
    }

    /// The program the service starts, split into words. `None` when the
    /// start has an unclosed quote; the rules refuse that before this runs.
    #[must_use]
    pub fn start_words(&self, app: &str) -> Option<(Program, Vec<String>)> {
        let (program, rest) = self.start(app)?;
        Some((program, split_args(&rest)?))
    }

    /// The apps that run a process, in name order.
    pub fn process_apps(&self) -> impl Iterator<Item = (&str, Manager)> {
        self.deploy
            .apps
            .iter()
            .filter(|(_, app)| app.process.is_some())
            .filter_map(|(name, _)| self.manager(name).map(|manager| (name.as_str(), manager)))
    }

    /// `name: env` pairs, for the screens.
    #[must_use]
    pub fn app_env(&self, app: &str) -> BTreeMap<String, String> {
        self.process(app)
            .map(|process| process.env.clone())
            .unwrap_or_default()
    }

    /// The placeholders' values for one release and app.
    #[must_use]
    pub fn values<'a>(
        &'a self,
        release: Option<&'a str>,
        version: Option<&'a str>,
        app: Option<&'a str>,
    ) -> Values<'a> {
        Values {
            root: self.dir(),
            name: self.name(),
            host: &self.endpoint.name,
            project: self.project.to_str().unwrap_or("."),
            config: self.config_dir.to_str().unwrap_or("."),
            release,
            version,
            app: app.map(|name| AppValues {
                name,
                source: self.source(name).display().to_string(),
                process: self.process(name).map(|_| self.process_name(name)),
            }),
        }
    }

    /// `text` with every placeholder replaced.
    ///
    /// # Errors
    /// Returns a message for a name with no value here. A deploy file is
    /// checked when it is read, so this means a bug in shipdeck.
    pub fn expand(
        &self,
        text: &str,
        release: Option<&str>,
        version: Option<&str>,
        app: Option<&str>,
    ) -> anyhow::Result<String> {
        placeholders::expand(text, &self.values(release, version, app))
            .map_err(|why| anyhow::anyhow!("{why}"))
    }

    /// The files a form writes at its path from the project folder, if it is
    /// written as a project path.
    #[must_use]
    pub fn is_project_path(text: &str) -> bool {
        paths::form(text) == Form::Project
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
