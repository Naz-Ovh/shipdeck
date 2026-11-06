//! Configuration for a deploy that ships several apps in one release.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Manager, Pipeline, Pm2, Service, SshTarget};

/// Release file used to restore the settings of every app on rollback.
pub const RELEASE_METADATA: &str = "shipdeck-monorepo.json";

/// One app's settings and paths after the deployment config is loaded.
pub struct ResolvedApp {
    pub name: String,
    pub project: std::path::PathBuf,
    pub release_dir: String,
    pub service: Service,
    pub pipeline: Pipeline,
}

/// Reads a monorepo config and reports YAML and shape problems.
///
/// # Errors
/// Returns the config problems when the document cannot be read.
pub fn parse(text: &str) -> std::result::Result<MonorepoConfig, Vec<super::Problem>> {
    let config: MonorepoConfig = parse_document(text)?;
    let problems = config.validate();
    if problems.is_empty() {
        Ok(config)
    } else {
        Err(problems)
    }
}

/// Reads one YAML document using its own config schema.
fn parse_document<T: JsonSchema + serde::de::DeserializeOwned>(
    text: &str,
) -> std::result::Result<T, Vec<super::Problem>> {
    let root = super::yaml::parse(text).map_err(|problem| vec![problem])?;
    let schema = schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value();
    let checked = super::check::structure_with(&root, &schema);
    let mut problems = checked.problems;
    match serde_json::from_value(checked.value) {
        Ok(config) if problems.is_empty() => Ok(config),
        Ok(_) => Err(problems),
        Err(err) => {
            if problems.is_empty() {
                problems.push(super::Problem::new(format!(
                    "config could not be read: {err}"
                )));
            }
            Err(problems)
        }
    }
}

impl MonorepoConfig {
    /// Loads the main config and any app files from a deploy folder.
    ///
    /// # Errors
    /// Returns the file problems if the config cannot be used.
    pub fn load(folder: &Path, path: &Path) -> std::result::Result<Self, super::FileProblems> {
        let text = std::fs::read_to_string(path).map_err(|err| {
            super::FileProblems::without_source(
                path.to_path_buf(),
                super::Problem::new(format!("cannot read it: {err}")),
            )
        })?;
        Self::from_text(folder, path, &text)
    }

    /// Parses a main config and loads its app files.
    ///
    /// # Errors
    /// Returns the file problems if the config cannot be used.
    pub fn from_text(
        folder: &Path,
        path: &Path,
        text: &str,
    ) -> std::result::Result<Self, super::FileProblems> {
        let failed = |problems| super::FileProblems {
            path: path.to_path_buf(),
            source: Some(text.to_string()),
            problems,
        };
        let mut config = parse(text).map_err(failed)?;
        config
            .load_app_files(folder)
            .map_err(|err| failed(vec![super::Problem::new(format!("{err:#}"))]))?;
        config
            .check_apps()
            .map_err(|err| failed(vec![super::Problem::new(format!("{err:#}"))]))?;
        Ok(config)
    }

    /// Resolves app paths and process settings before starting a deployment.
    ///
    /// # Errors
    /// Returns an error if an app is invalid or its project leaves the repository.
    pub fn resolve_apps(&self, folder: &Path) -> Result<Vec<ResolvedApp>> {
        self.check_apps()?;
        let root = folder
            .join(&self.project)
            .canonicalize()
            .context("opening the monorepo project folder")?;
        if !root.is_dir() {
            bail!("the monorepo project is not a folder");
        }
        let mut apps = Vec::with_capacity(self.apps.len());
        for (name, definition) in &self.apps {
            let AppDefinition::Inline(app) = definition else {
                bail!("app {name}: config file has not been loaded");
            };
            let project = root
                .join(&app.project)
                .canonicalize()
                .with_context(|| format!("opening the project folder for app {name}"))?;
            if !project.starts_with(&root) || !project.is_dir() {
                bail!("app {name}: project must be a folder inside the repository");
            }
            apps.push(ResolvedApp {
                name: name.clone(),
                project,
                release_dir: format!("apps/{name}"),
                service: app.service(self, name),
                pipeline: app.pipeline(&self.pipeline)?,
            });
        }
        Ok(apps)
    }

    /// Checks each resolved app using the same deployment rules as a service.
    ///
    /// # Errors
    /// Returns an error when an app is unresolved or cannot be deployed.
    pub fn check_apps(&self) -> Result<()> {
        let root = super::yaml::parse("{}").map_err(|problem| anyhow::anyhow!(problem.message))?;
        let mut pm2_names = std::collections::BTreeSet::new();
        for (name, definition) in &self.apps {
            let AppDefinition::Inline(app) = definition else {
                bail!("app {name}: config file has not been loaded");
            };
            let config = super::DeployConfig {
                name: self.name.clone(),
                project: app.project.clone(),
                ssh: self.ssh.clone(),
                service: app.service(self, name),
                pipeline: app.pipeline(&self.pipeline)?,
            };
            if config.service.manager == Manager::Pm2
                && !pm2_names.insert(config.service.pm2_app().to_string())
            {
                bail!("app {name}: PM2 app name is used by another app");
            }
            let problems = super::rules::check(&config, &root);
            if !problems.is_empty() {
                bail!(
                    "app {name}: {}",
                    problems
                        .iter()
                        .map(|problem| problem.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
        }
        Ok(())
    }

    /// Reads and checks the settings recorded in a release.
    ///
    /// # Errors
    /// Returns an error when the recorded settings are invalid or unresolved.
    pub fn from_release_metadata(data: &[u8]) -> Result<Self> {
        if u64::try_from(data.len())? > crate::remote::READ_CAP {
            bail!("release settings are too large to read during rollback");
        }
        let config: Self = serde_json::from_slice(data).context("reading release settings")?;
        let problems = config.validate();
        if !problems.is_empty() {
            bail!(
                "invalid release settings: {}",
                problems
                    .iter()
                    .map(|problem| problem.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        if config
            .apps
            .values()
            .any(|app| matches!(app, AppDefinition::File(_)))
        {
            bail!("release settings contain unresolved app files");
        }
        config.check_apps().context("invalid release settings")?;
        Ok(config)
    }

    /// Encodes the resolved settings stored with the shared release.
    ///
    /// # Errors
    /// Returns an error for unresolved app files or oversized metadata.
    pub fn release_metadata(&self) -> Result<Vec<u8>> {
        if self
            .apps
            .values()
            .any(|app| matches!(app, AppDefinition::File(_)))
        {
            bail!("app config files must be loaded before packing");
        }
        if !self.validate().is_empty() {
            bail!("cannot save invalid monorepo settings");
        }
        self.check_apps()
            .context("cannot save invalid app settings")?;
        let data = serde_json::to_vec(self).context("encoding release settings")?;
        if u64::try_from(data.len())? > crate::remote::READ_CAP {
            bail!("release settings are too large to read during rollback");
        }
        Ok(data)
    }

    /// Checks values used for process names and release paths.
    #[must_use]
    pub fn validate(&self) -> Vec<super::Problem> {
        let mut problems = Vec::new();
        if !super::is_unit_name(&self.name) || self.name.starts_with('.') {
            problems.push(super::Problem::new(
                "monorepo name must be a valid process name",
            ));
        }
        if !self.dir.starts_with('/')
            || self.dir.len() <= 1
            || !self
                .dir
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
            || self.dir.split('/').any(|part| part == "..")
        {
            problems.push(super::Problem::new(
                "dir must be a safe absolute path on the server",
            ));
        }
        if self.apps.is_empty() {
            problems.push(super::Problem::new("apps must contain at least one app"));
        }
        for (name, definition) in &self.apps {
            if !super::is_unit_name(name) || name.starts_with('.') {
                problems.push(super::Problem::new(format!(
                    "app {name:?} must be a valid process name"
                )));
            }
            if let AppDefinition::Inline(app) = definition {
                problems.extend(app.validate(name));
            }
        }
        problems
    }

    /// Reads app files relative to the main deploy file.
    ///
    /// # Errors
    /// Returns an error if an app file is unsafe, missing, or invalid.
    pub fn load_app_files(&mut self, folder: &Path) -> Result<()> {
        let root = folder
            .canonicalize()
            .with_context(|| format!("opening {}", folder.display()))?;
        for (name, definition) in &mut self.apps {
            let AppDefinition::File(file) = definition else {
                continue;
            };
            let relative = Path::new(file);
            if relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            {
                bail!("app {name}: {file} must be a path inside the deploy folder");
            }
            let app_file = root.join(relative);
            let path = app_file
                .canonicalize()
                .with_context(|| format!("opening app {name}: {file}"))?;
            if !path.starts_with(&root) {
                bail!("app {name}: {file} points outside the deploy folder");
            }
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let app = parse_document::<AppConfig>(&text).map_err(|problems| {
                anyhow::anyhow!(
                    "invalid app config {}: {}",
                    path.display(),
                    problems
                        .iter()
                        .map(|problem| problem.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?;
            let problems = app.validate(name);
            if !problems.is_empty() {
                bail!(
                    "invalid app config {}: {}",
                    path.display(),
                    problems
                        .iter()
                        .map(|problem| problem.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            self.app_files.push(app_file);
            if !self.app_files.contains(&path) {
                self.app_files.push(path);
            }
            *definition = AppDefinition::Inline(Box::new(app));
        }
        Ok(())
    }
}

/// The shared settings and app definitions of one monorepo deploy.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonorepoConfig {
    /// The deploy's name.
    pub name: String,
    /// The repository root, relative to deploy.yaml.
    #[serde(default = "super::default_project")]
    pub project: String,
    /// The server where every app runs.
    pub ssh: SshTarget,
    /// The server folder that holds the single release and its backups.
    pub dir: String,
    /// The default process manager for apps.
    #[serde(default)]
    pub process_manager: ProcessManager,
    /// Global pipeline settings inherited by apps.
    #[serde(default = "empty_pipeline")]
    #[schemars(with = "Pipeline")]
    pub pipeline: serde_json::Value,
    /// Named apps, inline or in a separate file.
    pub apps: BTreeMap<String, AppDefinition>,
    /// App config files, kept out of release archives.
    #[serde(skip)]
    pub app_files: Vec<std::path::PathBuf>,
}

/// An inline app or the path to its config file.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AppDefinition {
    /// Path relative to the main deploy.yaml.
    File(String),
    /// The app's settings in the main file.
    Inline(Box<AppConfig>),
}

/// Settings for one app in a shared release.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// App source folder, relative to the repository root.
    #[serde(default = "super::default_project")]
    pub project: String,
    /// Command that starts the app, or omit it when a binary is packed.
    #[serde(default)]
    pub start: Option<String>,
    /// Environment variables for the app.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Environment file inside the app's release folder.
    #[serde(default)]
    pub env_file: Option<String>,
    /// App folders kept across releases.
    #[serde(default)]
    pub shared: Vec<String>,
    /// Replaces the global process manager when present.
    #[serde(default)]
    pub process_manager: Option<ProcessManager>,
    /// Pipeline settings that replace the corresponding global settings.
    #[serde(default = "empty_pipeline")]
    #[schemars(with = "Pipeline")]
    pub pipeline: serde_json::Value,
}

/// The process manager and its own settings.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessManager {
    /// `systemd` by default, or `pm2`.
    #[serde(default)]
    pub name: Manager,
    /// The user that runs the app. Default: the SSH user.
    #[serde(default)]
    pub run_as: Option<String>,
    /// An optional app name for PM2.
    #[serde(default)]
    pub app: Option<String>,
    /// A PM2 ecosystem file inside the app's release folder.
    #[serde(default)]
    pub ecosystem: Option<String>,
    /// Additional PM2 app options.
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
}

impl AppConfig {
    /// Builds the process settings for this app in the shared deploy folder.
    #[must_use]
    pub fn service(&self, config: &MonorepoConfig, name: &str) -> Service {
        let manager = self.process_manager(&config.process_manager);
        Service {
            name: format!("{}-{name}", config.name),
            dir: config.dir.clone(),
            manager: manager.name,
            pm2: Pm2 {
                app: manager.app.clone(),
                ecosystem: manager.ecosystem.clone(),
                options: manager.options.clone(),
            },
            start: self.start.clone(),
            run_as: manager.run_as.clone(),
            env_file: self.env_file.clone(),
            env: self.env.clone(),
            shared: self.shared.clone(),
        }
    }

    /// Checks app values before they are used by a deploy.
    fn validate(&self, name: &str) -> Vec<super::Problem> {
        let mut problems = Vec::new();
        let project = Path::new(&self.project);
        if self.project.is_empty()
            || project.is_absolute()
            || project
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
        {
            problems.push(super::Problem::new(format!(
                "app {name}: project must be inside the repository"
            )));
        }
        if let Some(start) = &self.start {
            let program = start.split_whitespace().next().unwrap_or_default();
            if start.contains(['\n', '\r', '\\'])
                || super::split_args(start).is_none()
                || !super::is_program(program)
            {
                problems.push(super::Problem::new(format!(
                    "app {name}: start must be a valid command on one line"
                )));
            }
        }
        if self
            .env_file
            .as_deref()
            .is_some_and(|path| path.starts_with('/') || !super::is_program(path))
        {
            problems.push(super::Problem::new(format!(
                "app {name}: env_file must be a path inside the release"
            )));
        }
        for (key, value) in &self.env {
            let valid_key = !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid_key || value.contains(['"', '\\', '\n', '\r']) {
                problems.push(super::Problem::new(format!(
                    "app {name}: invalid environment variable {key:?}"
                )));
            }
        }
        for (index, path) in self.shared.iter().enumerate() {
            if path.is_empty()
                || Path::new(path)
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                || self.shared[..index].iter().any(|other| {
                    Path::new(path).starts_with(other) || Path::new(other).starts_with(path)
                })
            {
                problems.push(super::Problem::new(format!(
                    "app {name}: invalid or overlapping shared folder {path:?}"
                )));
            }
        }
        problems
    }

    /// The app's manager replaces the global manager as a whole.
    #[must_use]
    pub fn process_manager<'a>(&'a self, global: &'a ProcessManager) -> &'a ProcessManager {
        self.process_manager.as_ref().unwrap_or(global)
    }

    /// Resolves an app's pipeline settings over the global pipeline.
    ///
    /// # Errors
    /// Returns an error when the combined pipeline is invalid.
    pub fn pipeline(&self, global: &serde_json::Value) -> Result<Pipeline> {
        let mut pipeline = global.clone();
        override_pipeline(&mut pipeline, &self.pipeline);
        serde_json::from_value(pipeline).context("reading the app's pipeline")
    }
}

/// Starts an omitted pipeline as an empty map.
fn empty_pipeline() -> serde_json::Value {
    serde_json::json!({})
}

/// Replaces explicitly supplied app settings, keeping other global settings.
fn override_pipeline(global: &mut serde_json::Value, app: &serde_json::Value) {
    if let (serde_json::Value::Object(global), serde_json::Value::Object(app)) = (&mut *global, app)
    {
        for (key, value) in app {
            if let Some(existing) = global.get_mut(key) {
                override_pipeline(existing, value);
            } else {
                global.insert(key.clone(), value.clone());
            }
        }
    } else {
        *global = app.clone();
    }
}

#[cfg(test)]
#[path = "monorepo_test.rs"]
mod tests;
