//! Preflight: what your PC and the server need before anything runs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use super::commands;
use super::report::{LogKind, Reporter, Unit};
use super::run::{Ended, Run};
use super::steps::{Step, all_scopes};
use crate::build;
use crate::config::{Deployment, Manager, ManagerKind, Program};
use crate::human;
use crate::remote::{Facts, PM2_NEWEST_MAJOR, ReleaseEntry, Server, parse_version};
use crate::ssh::{Pool, ShellEnv};

/// Below this much free space on the server, preflight stops.
pub(super) const MIN_FREE: u64 = 100 * 1024 * 1024;
/// What shipdeck itself runs on the server, with the package each comes in.
pub(super) const SERVER_TOOLS: [(&str, &str); 3] =
    [("tar", "tar"), ("zstd", "zstd"), ("sha256sum", "coreutils")];

impl Run<'_> {
    /// Preflight: checks, then your commands.
    pub(super) async fn step_preflight(&mut self, pool: &Pool) -> Result<(Server, Facts), Ended> {
        self.start(Step::Preflight, "checking your PC and the server");
        let found = Box::pin(self.preflight(pool)).await;
        match found {
            Ok((server, facts, summary)) => {
                self.finish(Step::Preflight, summary);
                Ok((server, facts))
            }
            Err(err) => Err(self.stop(Step::Preflight, &err)),
        }
    }

    async fn preflight(&mut self, pool: &Pool) -> Result<(Server, Facts, String)> {
        self.gates()?;
        self.check_local()?;
        let endpoint = &self.deployment.endpoint;
        let address = if endpoint.is_alias() {
            format!(" -> {}", endpoint.address())
        } else {
            String::new()
        };
        self.report.log(
            LogKind::Command,
            format!(
                "connect {}{address} ({})",
                endpoint.label(),
                pool.login().describe()
            ),
        );
        let session = pool
            .get_with(
                endpoint,
                |what| self.report.log(LogKind::Info, what),
                |done, total| {
                    self.report.progress(
                        self.at(Step::Preflight),
                        done,
                        total,
                        Unit::Bytes,
                        "uploading the agent",
                    );
                },
            )
            .await?;
        log_shell_env(self.report, &session.shell_env);
        self.session = Some(Arc::clone(&session));
        let base = self.base_server(Arc::clone(&session))?;
        let tools = self.tools();
        let mut facts = base.facts(&tools).await?;
        let endpoint = &self.deployment.endpoint;
        let dir = self.deployment.dir();
        if !facts.dir_exists {
            self.report.log(
                LogKind::Info,
                format!("first deploy of {}: creating {dir}", self.deployment.name()),
            );
            base.create_app_dir(self.report).await?;
        } else if !facts.dir_writable {
            bail!("{} cannot write to {dir}", endpoint.user);
        }
        self.lock_and_refresh(&base, &mut facts).await?;
        if facts.free_bytes.is_some_and(|free| free < MIN_FREE) {
            bail!("only {} free on the server", human::bytes(MIN_FREE));
        }
        self.check_tools(&facts)?;
        self.check_pm2(&facts)?;
        self.start_paths(&facts);
        self.previous.clone_from(&facts.current);

        let summary = format!(
            "{} · {} · {} free",
            facts.arch,
            facts
                .glibc
                .as_ref()
                .map_or_else(|| "no glibc".into(), |v| format!("glibc {v}")),
            facts
                .free_bytes
                .map_or_else(|| "unknown".into(), human::bytes)
        );
        self.report.log(
            LogKind::Info,
            format!(
                "live: {} · service {}",
                facts.current.as_deref().unwrap_or("nothing deployed yet"),
                if facts.active.is_empty() {
                    "unknown"
                } else {
                    &facts.active
                }
            ),
        );
        let exec = self.exec(Step::Preflight);
        Box::pin(exec.step(Step::Preflight, None)).await?;
        Ok((base, facts, summary))
    }

    /// Takes the project lock, which needs the folder, then reads the live
    /// facts again: what was read before the lock may have changed.
    async fn lock_and_refresh(&mut self, base: &Server, facts: &mut Facts) -> Result<()> {
        self.project_lock = Some(crate::agent::ProjectGuard::acquire(base).await?);
        self.check_lock()?;
        let fresh = base.facts(&[]).await?;
        facts.current = fresh.current;
        facts.content = fresh.content;
        facts.active = fresh.active;
        facts.systemd = fresh.systemd;
        facts.free_bytes = fresh.free_bytes;
        Ok(())
    }

    /// Refuses a deploy that uses a manager shipdeck does not manage yet.
    fn gates(&self) -> Result<()> {
        for app in self.deployment.deploy.apps.values() {
            if app.nginx.is_some() {
                bail!("nginx management is not built yet");
            }
        }
        if self
            .deployment
            .process_apps()
            .any(|(_, manager)| manager.kind() == ManagerKind::Oxmgr)
        {
            bail!("oxmgr is not built yet");
        }
        Ok(())
    }

    /// What can be checked on this machine before connecting.
    fn check_local(&self) -> Result<()> {
        if !self.deployment.project.is_dir() {
            bail!(
                "the project folder {} does not exist",
                self.deployment.project.display()
            );
        }
        for name in self.deployment.deploy.apps.keys() {
            let source = self.deployment.source(name);
            if !source.is_dir() {
                bail!(
                    "app {name}: the source folder {} does not exist",
                    source.display()
                );
            }
        }
        for name in self.deployment.process_apps().map(|(name, _)| name) {
            if let Some(template) = self.deployment.unit_template(name)
                && !template.is_file()
            {
                bail!(
                    "app {name}: the unit template {} does not exist",
                    template.display()
                );
            }
        }
        for (step, scope) in all_scopes(self.deployment) {
            for runner in commands::runners(scope.local) {
                if runner.key() == "sh" && !cfg!(windows) && build::find_program("sh").is_none() {
                    return Err(commands::missing_local(runner, step, scope.app));
                }
                if runner.key() != "sh" && build::find_program(runner.key()).is_none() {
                    return Err(commands::missing_local(runner, step, scope.app));
                }
            }
        }
        Ok(())
    }

    /// Every program the server must have.
    fn tools(&self) -> Vec<String> {
        let mut tools: BTreeSet<String> = SERVER_TOOLS
            .iter()
            .map(|(tool, _)| (*tool).into())
            .collect();
        tools.extend(self.deployment.steps().preflight.requires.iter().cloned());
        for (_, scope) in all_scopes(self.deployment) {
            for runner in commands::runners(scope.server) {
                tools.insert(runner.key().to_string());
            }
        }
        for name in self.deployment.deploy.apps.keys() {
            match self.deployment.manager(name) {
                Some(Manager::Pm2(_)) => {
                    tools.insert("pm2".into());
                    if self.deployment.pm2_ecosystem(name).is_none()
                        && let Some((Program::Tool(tool), _)) = self.deployment.start(name)
                    {
                        tools.insert(tool);
                    }
                }
                Some(Manager::Systemd(_)) => {
                    if let Some((Program::Tool(tool), _)) = self.deployment.start(name) {
                        tools.insert(tool);
                    }
                }
                _ => {}
            }
            if self
                .deployment
                .app(name)
                .is_some_and(|app| app.steps.health.url.is_some())
            {
                tools.insert("curl".into());
                tools.insert("wget".into());
            }
        }
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.agent().is_none())
        {
            tools.insert("flock".into());
        }
        tools.into_iter().collect()
    }

    /// Every program is there, and a missing runner names its step and app.
    fn check_tools(&self, facts: &Facts) -> Result<()> {
        let mut ours = Vec::new();
        let mut missing = Vec::new();
        for (tool, path) in &facts.tools {
            let package = SERVER_TOOLS
                .iter()
                .find(|(name, _)| name == tool)
                .map(|(_, package)| *package);
            match (path, package) {
                (Some(path), _) => self.report.log(LogKind::Success, format!("{tool}: {path}")),
                (None, Some(package)) => ours.push((tool.as_str(), package)),
                (None, None) => missing.push(tool.as_str()),
            }
        }
        if !ours.is_empty() {
            let tools: Vec<&str> = ours.iter().map(|(tool, _)| *tool).collect();
            let packages = ours
                .iter()
                .map(|(_, package)| *package)
                .collect::<Vec<_>>()
                .join(" ");
            bail!(
                "the server has no {}, which shipdeck unpacks and checks releases with; install {} \
                 (apt install {packages}, or dnf install {packages})",
                tools.join(" or "),
                if ours.len() == 1 { "it" } else { "them" }
            );
        }
        if !missing.is_empty() {
            let http = self
                .deployment
                .deploy
                .apps
                .values()
                .any(|app| app.steps.health.url.is_some());
            let missing: Vec<&str> = missing
                .into_iter()
                .filter(|tool| !(http && matches!(*tool, "curl" | "wget")))
                .collect();
            if !missing.is_empty() {
                bail!(
                    "{} cannot find {} on the server; install {} or list it in preflight.requires",
                    self.deployment.endpoint.user,
                    missing.join(", "),
                    if missing.len() == 1 { "it" } else { "them" }
                );
            }
        }
        if self
            .deployment
            .deploy
            .apps
            .values()
            .any(|app| app.steps.health.url.is_some())
            && facts.tool_path("curl").is_none()
            && facts.tool_path("wget").is_none()
        {
            bail!("HTTP health checks require curl or wget on the server");
        }
        if self
            .deployment
            .process_apps()
            .any(|(_, manager)| manager.kind() == ManagerKind::Systemd)
        {
            for name in self.deployment.process_apps().map(|(name, _)| name) {
                if let Some((Program::Tool(tool), _)) = self.deployment.start(name)
                    && let Some(path) = facts.tool_path(&tool)
                    && (path.starts_with("/home/") || path.starts_with("/root/"))
                {
                    bail!(
                        "{tool} is at {path}, but the built-in unit hides home folders from the \
                         service (ProtectHome=true). Install {tool} system-wide, or add a \
                         process.systemd.template without ProtectHome"
                    );
                }
            }
        }
        Ok(())
    }

    fn check_pm2(&self, facts: &Facts) -> Result<()> {
        let Some(version) = &facts.pm2_version else {
            return Ok(());
        };
        let (version, major) = parse_version(version).map_err(|why| anyhow!(why))?;
        if major > PM2_NEWEST_MAJOR {
            self.report.log(
                LogKind::Warn,
                format!(
                    "pm2 {version} is newer than this shipdeck is tested with \
                     ({PM2_NEWEST_MAJOR}.x); deploying anyway"
                ),
            );
        } else {
            self.report.log(LogKind::Success, format!("pm2 {version}"));
        }
        for name in self.deployment.deploy.apps.keys() {
            if self.deployment.manager(name).map(|m| m.kind()) == Some(ManagerKind::Pm2) {
                let process = self.deployment.process_name(name);
                if matches!(
                    facts.systemd.as_str(),
                    "active" | "activating" | "reloading"
                ) {
                    bail!(
                        "the systemd service {process} is still running and would clash with PM2; \
                         stop it first: sudo systemctl disable --now {process}"
                    );
                }
            }
        }
        Ok(())
    }

    /// Where preflight found each start program, for the unit and PM2 file.
    fn start_paths(&mut self, facts: &Facts) {
        let mut paths = BTreeMap::new();
        for name in self.deployment.deploy.apps.keys() {
            if let Some((Program::Tool(tool), _)) = self.deployment.start(name)
                && let Some(path) = facts.tool_path(&tool)
            {
                paths.insert(name.clone(), path.to_string());
            }
        }
        self.start_paths = paths;
    }

    /// The deployment folder's handle.
    fn base_server(&self, session: Arc<crate::ssh::Session>) -> Result<Server> {
        match self.deployment.first_process() {
            Some(name) => Server::for_app(self.deployment, name, session),
            None => Ok(Server::new(session, self.deployment.dir().to_string())),
        }
    }

    /// Connects for a rollback and reads the target release.
    pub(super) async fn rollback_preflight(
        &mut self,
        pool: &Pool,
        target: &str,
    ) -> Result<(), Ended> {
        self.start(Step::Preflight, "checking the server");
        let checked = Box::pin(self.rollback_check(pool, target)).await;
        match checked {
            Ok(()) => {
                self.finish(Step::Preflight, format!("release {target}"));
                Ok(())
            }
            Err(err) => Err(self.stop(Step::Preflight, &err)),
        }
    }

    async fn rollback_check(&mut self, pool: &Pool, target: &str) -> Result<()> {
        let endpoint = &self.deployment.endpoint;
        self.report
            .log(LogKind::Command, format!("connect {}", endpoint.label()));
        let session = pool.get(endpoint).await?;
        self.session = Some(Arc::clone(&session));
        let base = self.base_server(Arc::clone(&session))?;
        if !base.facts(&[]).await?.dir_exists {
            bail!("{} does not exist on the server", self.deployment.dir());
        }
        self.project_lock = Some(crate::agent::ProjectGuard::acquire(&base).await?);
        let listing = base.list().await?;
        let Some(entry) = listing.entries.iter().find(|entry| entry.id == target) else {
            bail!("{target} is no longer on the server");
        };
        if entry.current {
            bail!("{target} is already the current release");
        }
        let settings = base
            .settings(target, entry_source(entry))
            .await
            .map_err(|err| {
                anyhow!("{err:#}\nthis release was made before config format 2; deploy again")
            })?;
        let restored = Deployment::from_settings(
            &settings,
            &self.deployment.project,
            &self.deployment.config_dir,
            self.deployment.file.clone(),
            self.deployment.slug.clone(),
        )
        .map_err(|problem| anyhow!("{problem}"))?;
        if restored.deploy.apps.values().any(|app| app.nginx.is_some()) {
            bail!("nginx management is not built yet");
        }
        if restored
            .process_apps()
            .any(|(_, manager)| manager.kind() == ManagerKind::Oxmgr)
        {
            bail!("oxmgr is not built yet");
        }
        self.previous = listing.current;
        self.release = Some(target.to_string());
        self.version = crate::release::ReleaseName::parse(target).map(|name| name.version);
        self.from_backup = entry.kind == crate::remote::Kind::Compressed;
        Ok(())
    }
}

/// Whether a release is read from its folder or its backup.
fn entry_source(entry: &ReleaseEntry) -> crate::remote::Source {
    match entry.kind {
        crate::remote::Kind::Open => crate::remote::Source::Folder,
        crate::remote::Kind::Compressed => crate::remote::Source::Backup,
    }
}

fn log_shell_env(report: &Reporter, shell_env: &ShellEnv) {
    match shell_env {
        ShellEnv::Loaded => report.log(
            LogKind::Info,
            "server commands get the login shell's environment (server.ssh.shell_env)",
        ),
        ShellEnv::Off => report.log(
            LogKind::Info,
            "server commands run without the login shell's startup files",
        ),
        ShellEnv::Failed(why) => report.log(
            LogKind::Warn,
            format!("{why}; server commands run without them"),
        ),
    }
}
