//! Putting a release back: after a failure in activation, or by hand.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use super::commands::Exec;
use super::report::{LogKind, Outcome};
use super::run::{Ended, Run};
use super::steps::{self, Action, Step};
use crate::bundle;
use crate::config::{Deployment, RollbackMode};
use crate::release::ReleaseName;
use crate::remote::{Kind, Server, Source};

impl Run<'_> {
    /// After a failed switch, restart, nginx or health check: journal, ask
    /// (or not) and put the previous release back.
    pub(super) async fn recover(&mut self, step: Step, err: anyhow::Error) -> Ended {
        self.failed_step = Some(step);
        self.report.fail(self.at(step), super::short(&err));
        self.report.log(LogKind::Error, format!("{err:#}"));
        if self
            .session
            .as_ref()
            .is_none_or(|session| session.is_closed())
            || self
                .project_lock
                .as_ref()
                .is_some_and(|guard| !guard.is_alive())
        {
            return self.connection_lost(step, &err);
        }
        self.show_journal().await;
        let error = format!("{err:#}");
        let id = self.release.clone().unwrap_or_default();
        let Some(previous) = self.previous.clone().filter(|previous| *previous != id) else {
            self.report.log(
                LogKind::Warn,
                "nothing was live before, so there is nothing to roll back to",
            );
            self.skip_rest(step, "stopped");
            return Ended {
                outcome: Outcome::Failed {
                    error: error.clone(),
                },
                failed: Some((step, error)),
            };
        };
        if !self.should_roll_back(step, &err, &previous).await {
            self.report.log(
                LogKind::Warn,
                format!(
                    "{id} stays live as you chose; roll back any time from the Releases screen"
                ),
            );
            self.skip_rest(step, "kept as it is");
            return Ended {
                outcome: Outcome::Kept {
                    release: id,
                    error: error.clone(),
                },
                failed: Some((step, error)),
            };
        }
        self.skip_rest(step, "rolled back");
        self.start(Step::Rollback, &format!("rolling back to {previous}"));
        let restored = Box::pin(self.restore_back(&previous, &id)).await;
        match restored {
            Ok(()) => {
                self.discard_failed(&id).await;
                self.report
                    .log(LogKind::Success, format!("{previous} is live again"));
                self.finish(Step::Rollback, format!("current -> {previous}"));
                Ended {
                    outcome: Outcome::RolledBack {
                        error: error.clone(),
                        to: previous,
                    },
                    failed: Some((step, error)),
                }
            }
            Err(rollback) => {
                self.report
                    .log(LogKind::Error, format!("rollback failed: {rollback:#}"));
                self.fail(Step::Rollback, super::short(&rollback));
                Ended {
                    outcome: Outcome::RollbackFailed {
                        error: error.clone(),
                        rollback_error: format!("{rollback:#}"),
                    },
                    failed: Some((step, error)),
                }
            }
        }
    }

    /// Runs the steps that put `previous` back, as an automatic rollback.
    ///
    /// The rollback commands come from the deploy file shipdeck was started
    /// with, and run before the action, exactly as they do in a rollback by
    /// hand. `SHIPDECK_PREVIOUS` names the release being put back.
    async fn restore_back(&self, previous: &str, failed: &str) -> anyhow::Result<()> {
        self.exec_rollback_commands(previous)
            .step(Step::Rollback, None)
            .await?;
        self.check_lock()?;
        let restored = self.restored(previous).await?;
        let rows = self
            .steps
            .iter()
            .map(|step| (*step, self.at(Step::Rollback)))
            .collect();
        Restore {
            deployment: &restored,
            report: self.report,
            session: self.session.clone(),
            target: previous.to_string(),
            version: ReleaseName::parse(previous).map(|name| name.version),
            previous: Some(failed.to_string()),
            from_backup: self.from_backup,
            rows,
            mark: false,
        }
        .run()
        .await
    }

    /// Removes the failed release unless `keep_failed` says otherwise.
    async fn discard_failed(&self, id: &str) {
        if self.deployment.keep_failed() {
            return;
        }
        let Some(server) = self.base_handle() else {
            return;
        };
        if let Err(discard) = server.discard(id, self.report).await {
            self.report.log(
                LogKind::Warn,
                format!("could not remove the failed release {id}: {discard:#}"),
            );
        }
    }

    /// Runs `failed`: the server's commands when it can be reached, then the
    /// local ones. Its own failure is only logged.
    pub(super) async fn failed(&mut self, ended: &Ended) {
        let Some((step, error)) = ended.failed.clone() else {
            self.skip(Step::Failed, "nothing failed");
            return;
        };
        self.failed_step = Some(step);
        self.start(
            Step::Failed,
            &format!("running failed commands (after {})", step.key()),
        );
        let reachable = self
            .session
            .as_ref()
            .is_some_and(|session| !session.is_closed())
            && self
                .project_lock
                .as_ref()
                .is_none_or(crate::agent::ProjectGuard::is_alive);
        let exec = self.exec(Step::Failed);
        let mut detail = String::new();
        let ran = if reachable {
            exec.step(Step::Failed, Some(&error)).await
        } else {
            super::commands::skipped(self.report, "the server cannot be reached");
            Box::pin(async {
                for scope in steps::scopes(self.deployment, Step::Failed) {
                    exec.local(Step::Failed, scope, Some(&error)).await?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
        };
        match ran {
            Ok(()) => {
                detail.push_str("done");
                self.finish(Step::Failed, detail);
            }
            Err(err) => {
                self.report
                    .log(LogKind::Warn, format!("failed commands: {err:#}"));
                self.warn(Step::Failed, super::short(&err));
            }
        }
    }

    /// The rollback steps after the target's settings are known.
    pub(super) async fn step_put_back(&mut self, target: &str) -> Result<()> {
        self.start(Step::Rollback, &format!("putting back {target}"));
        let restored = self.restored(target).await?;
        let ran = Box::pin(async {
            // The rollback commands come from the deploy file shipdeck was
            // started with, and see the release being put back as PREVIOUS.
            let exec = self.exec_rollback_commands(target);
            exec.step(Step::Rollback, None).await?;
            self.check_lock()?;
            if self.from_backup {
                self.record("backup_restored", target).await;
            }
            let rows = self
                .steps
                .iter()
                .map(|step| (*step, self.at(*step)))
                .collect();
            Restore {
                deployment: &restored,
                report: self.report,
                session: self.session.clone(),
                target: target.to_string(),
                version: self.version.clone(),
                previous: self.previous.clone(),
                from_backup: self.from_backup,
                rows,
                mark: true,
            }
            .run()
            .await?;
            Ok::<_, anyhow::Error>(())
        })
        .await;
        match ran {
            Ok(()) => {
                self.finish(Step::Rollback, format!("current -> {target}"));
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn exec_rollback_commands(&self, target: &str) -> Exec<'_> {
        let mut exec = self.exec(Step::Rollback);
        exec.previous = Some(target.to_string());
        exec
    }

    /// Reads a release's own settings and resolves them again.
    pub(super) async fn restored(&self, id: &str) -> Result<Deployment> {
        let server = self
            .base_handle()
            .ok_or_else(|| anyhow!("there is no server connection"))?;
        let listing = server.list().await?;
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| anyhow!("{id} is no longer on the server"))?;
        let source = match entry.kind {
            Kind::Open => Source::Folder,
            Kind::Compressed => Source::Backup,
        };
        let settings = server
            .settings(id, source)
            .await
            .with_context(|| "this release was made before config format 2; deploy again")?;
        Deployment::from_settings(
            &settings,
            &self.deployment.project,
            &self.deployment.config_dir,
            self.deployment.file.clone(),
            self.deployment.slug.clone(),
        )
        .map_err(|problem| anyhow!("{problem}"))
    }

    /// `rollback.mode`: auto rolls back; ask waits for an answer.
    pub(super) async fn should_roll_back(
        &self,
        step: Step,
        err: &anyhow::Error,
        previous: &str,
    ) -> bool {
        match self.deployment.rollback_mode() {
            RollbackMode::Auto => true,
            RollbackMode::Off => false,
            RollbackMode::Ask => {
                let id = self.release.clone().unwrap_or_default();
                let seconds = self.deployment.ask_secs();
                self.report.log(
                    LogKind::Info,
                    format!(
                        "asking whether to roll back to {previous}; with no answer in {seconds}s it rolls back"
                    ),
                );
                let answer = self.report.ask_rollback(
                    &id,
                    previous,
                    step.name().to_string(),
                    &super::short(err),
                    seconds,
                );
                match tokio::time::timeout(Duration::from_secs(seconds), answer).await {
                    Ok(Ok(true)) => {
                        self.report.log(LogKind::Info, "you chose to roll back");
                        true
                    }
                    Ok(Ok(false)) => {
                        self.report
                            .log(LogKind::Warn, format!("you chose to keep {id}"));
                        false
                    }
                    Ok(Err(_)) => true,
                    Err(_) => {
                        self.report.log(
                            LogKind::Warn,
                            format!("no answer in {seconds}s; rolling back"),
                        );
                        true
                    }
                }
            }
        }
    }

    /// A manual rollback that fails ends as "rollback failed".
    pub(super) fn stop_rollback(&mut self, step: Step, err: &anyhow::Error) -> Ended {
        self.failed_step = Some(step);
        self.report.fail(self.at(step), super::short(err));
        self.report.log(LogKind::Error, format!("{err:#}"));
        self.skip_rest(step, "rollback failed");
        Ended {
            outcome: Outcome::RollbackFailed {
                error: "the rollback failed".to_string(),
                rollback_error: format!("{err:#}"),
            },
            failed: Some((step, format!("{err:#}"))),
        }
    }

    fn connection_lost(&self, step: Step, err: &anyhow::Error) -> Ended {
        let error = format!("{err:#}");
        self.skip_rest(step, "connection lost; inspect the server before recovery");
        Ended {
            outcome: Outcome::RollbackFailed {
                error: error.clone(),
                rollback_error:
                    "connection lost during activation; inspect the live release and services"
                        .to_string(),
            },
            failed: Some((step, error)),
        }
    }

    async fn show_journal(&self) {
        let Some(name) = self.deployment.first_process() else {
            return;
        };
        let Some(server) = self.server_for_app(name) else {
            return;
        };
        if server.is_static() {
            return;
        }
        match server.journal().await {
            Ok(lines) if !lines.is_empty() => {
                self.report.log(LogKind::Info, "last log lines:");
                for line in lines {
                    self.report.log(LogKind::Err, line);
                }
            }
            _ => self
                .report
                .log(LogKind::Warn, "could not read the service's log"),
        }
    }

    fn server_for_app(&self, name: &str) -> Option<Server> {
        let session = self.session.clone()?;
        Server::for_app(self.deployment, name, session).ok()
    }

    fn base_handle(&self) -> Option<Server> {
        let session = self.session.clone()?;
        Some(Server::new(session, self.deployment.dir().to_string()))
    }
}

/// The steps of putting a release back, shared by an automatic and a manual
/// rollback.
struct Restore<'a> {
    deployment: &'a Deployment,
    report: &'a super::report::Reporter,
    session: Option<Arc<crate::ssh::Session>>,
    target: String,
    version: Option<String>,
    previous: Option<String>,
    from_backup: bool,
    rows: Vec<(Step, usize)>,
    mark: bool,
}

impl Restore<'_> {
    fn row(&self, step: Step) -> usize {
        self.rows
            .iter()
            .find(|(planned, _)| *planned == step)
            .map_or(0, |(_, row)| *row)
    }

    fn exec(&self, step: Step) -> Exec<'_> {
        Exec {
            deployment: self.deployment,
            report: self.report,
            session: self.session.clone(),
            release: Some(self.target.clone()),
            version: self.version.clone(),
            previous: self.previous.clone(),
            failed_step: None,
            action: Action::Rollback,
            row: self.row(step),
        }
    }

    fn start(&self, step: Step, what: &str) {
        if self.mark {
            self.report
                .start(self.row(step), &format!("{}: {what}", step.name()));
        }
    }

    fn finish(&self, step: Step, detail: impl Into<String>) {
        if self.mark {
            let detail = detail.into();
            self.report.finish(
                self.row(step),
                if detail.is_empty() {
                    "done".to_string()
                } else {
                    detail
                },
            );
        }
    }

    async fn run(&self) -> Result<()> {
        let server = self
            .exec(Step::Rollback)
            .server_for(None)
            .context("there is no server connection")?;
        if self.from_backup {
            server.restore(&self.target, self.report).await?;
            let links: Vec<String> = bundle::shared_links(self.deployment)
                .into_iter()
                .map(|link| link.path)
                .collect();
            server
                .link_shared(&self.target, &links, self.report)
                .await?;
        }
        self.start(Step::Setup, "running the restored release's setup");
        if self.from_backup {
            for scope in steps::scopes(self.deployment, Step::Setup) {
                self.exec(Step::Setup)
                    .server(Step::Setup, scope, None)
                    .await?;
            }
            self.finish(Step::Setup, "setup.server ran again");
        } else {
            self.report.log(
                LogKind::Info,
                "an open release keeps what its setup made; it is not run again",
            );
            self.finish(Step::Setup, "already set up");
        }
        self.start(Step::Switch, "making it current again");
        server.switch(&self.target, self.report).await?;
        for name in self.deployment.process_apps().map(|(name, _)| name) {
            self.exec(Step::Switch)
                .server_for(Some(name))?
                .install_unit(self.report)
                .await?;
        }
        self.finish(Step::Switch, format!("current -> {}", self.target));
        self.start(Step::Restart, "restarting the processes");
        let names: Vec<String> = self
            .deployment
            .process_apps()
            .map(|(name, _)| name.to_string())
            .collect();
        for name in &names {
            self.exec(Step::Restart)
                .server_for(Some(name))?
                .restart(self.report)
                .await?;
        }
        self.finish(
            Step::Restart,
            if names.is_empty() {
                "static apps: nothing to restart".to_string()
            } else {
                crate::config::count(names.len(), "process")
            },
        );
        self.start(Step::Nginx, "checking nginx");
        self.finish(Step::Nginx, "nothing to configure");
        self.start(Step::Health, "checking the restored release");
        self.health().await?;
        self.finish(Step::Health, "checked");
        self.start(Step::Done, "running the restored release's done commands");
        self.exec(Step::Done).step(Step::Done, None).await?;
        self.finish(Step::Done, "");
        Ok(())
    }

    /// The restored release's own health checks.
    async fn health(&self) -> Result<()> {
        let timeout = Duration::from_secs(self.deployment.health_timeout());
        let deadline = tokio::time::Instant::now() + timeout;
        for name in self.deployment.deploy.apps.keys() {
            let all = steps::scopes(self.deployment, Step::Health);
            let scope = all
                .iter()
                .find(|scope| scope.app == Some(name.as_str()))
                .copied()
                .ok_or_else(|| anyhow!("app {name} has no health step"))?;
            let url = self
                .deployment
                .app_health(name)
                .and_then(|health| health.url.clone())
                .map(|url| {
                    self.deployment.expand(
                        &url,
                        Some(&self.target),
                        self.version.as_deref(),
                        Some(name),
                    )
                })
                .transpose()?;
            loop {
                let exec = self.exec(Step::Health);
                let tried = async {
                    exec.server(Step::Health, scope, None).await?;
                    if let Some(url) = &url {
                        exec.server_for(Some(name))?.fetch(url).await?;
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                match tried {
                    Ok(()) => break,
                    Err(err) => {
                        if tokio::time::Instant::now() + Duration::from_secs(1) >= deadline {
                            bail!(
                                "app {name} was not healthy within {}s: {err:#}",
                                timeout.as_secs()
                            );
                        }
                        self.report
                            .log(LogKind::Warn, format!("app {name}: {err:#}; trying again"));
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
        let all = steps::scopes(self.deployment, Step::Health);
        let release = all
            .iter()
            .find(|scope| scope.app.is_none())
            .copied()
            .ok_or_else(|| anyhow!("the release has no health step"))?;
        if !release.server.is_empty() {
            loop {
                match self
                    .exec(Step::Health)
                    .server(Step::Health, release, None)
                    .await
                {
                    Ok(()) => break,
                    Err(err) => {
                        if tokio::time::Instant::now() + Duration::from_secs(1) >= deadline {
                            bail!(
                                "the health commands failed within {}s: {err:#}",
                                timeout.as_secs()
                            );
                        }
                        self.report
                            .log(LogKind::Warn, format!("health: {err:#}; trying again"));
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
        Ok(())
    }
}
