//! A deploy runs the pipeline: preflight, build, pack, upload, unpack,
//! setup, backup, switch, restart, health and done, then failed if a step
//! failed. A failure after the switch asks whether to put the previous
//! release back.

pub(crate) mod apps;
mod backup;
mod health;
mod multiple;
mod multiple_hooks;
mod multiple_recover;
mod multiple_release;
mod preflight;
mod recover;
pub mod report;
mod rollback;
mod ship;
mod stages;
mod steps;
mod util;

pub use multiple::{deploy_monorepo, monorepo_steps, rollback_monorepo};
pub use preflight::{AppServer, preflight_apps};
pub use rollback::{rollback, rollback_step_names, rollback_steps};

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::build;
use crate::bundle;
use crate::bundle::files::{self, Kind, Source};
use crate::bundle::{Bundle, Contents};
use crate::config::{Commands, DeployConfig, Deployable, release_path};
use crate::human;
use crate::release::{Manifest, ReleaseName};
use crate::remote::{CommandEnv, Facts, Place, Server};
use crate::ssh::Pool;
use backup::restore;
use preflight::{check_build, preflight, start_path};
use report::{LogKind, Outcome, Reporter, Unit};
pub use steps::{Action, Step, deploy_step_names, deploy_steps};
use util::{Throttle, count, restart_processes, short, some, switch_processes};

pub async fn deploy(deployable: Deployable, pool: Arc<Pool>, report: Reporter) {
    let mut run = Run::new(
        &deployable,
        &report,
        deploy_steps(&deployable.config),
        Action::Deploy,
    );
    let ended = Box::pin(run.deploy(&pool)).await;
    Box::pin(run.failed(&ended)).await;
    run.unlock_project().await;
    report.done(ended.outcome);
}

/// How a run ended, and the step that failed if one did.
pub(crate) struct Ended {
    outcome: Outcome,
    failed: Option<(Step, String)>,
}

impl Ended {
    fn ok(release: &str) -> Self {
        Self {
            outcome: Outcome::Succeeded {
                release: release.to_string(),
            },
            failed: None,
        }
    }
}

/// One deploy or rollback in progress: its steps and what is known so far.
pub(crate) struct Run<'a> {
    deployable: &'a Deployable,
    report: &'a Reporter,
    steps: Vec<Step>,
    action: Action,
    /// The release this run puts live, once known.
    release: Option<String>,
    version: Option<String>,
    /// The release that was live before.
    previous: Option<String>,
    project_lock: Option<crate::agent::ProjectGuard>,
}

impl<'a> Run<'a> {
    pub(crate) fn new(
        deployable: &'a Deployable,
        report: &'a Reporter,
        steps: Vec<Step>,
        action: Action,
    ) -> Self {
        Self {
            deployable,
            report,
            steps,
            action,
            release: None,
            version: None,
            previous: None,
            project_lock: None,
        }
    }

    fn config(&self) -> &'a DeployConfig {
        &self.deployable.config
    }

    async fn unlock_project(&mut self) {
        if let Some(guard) = self.project_lock.take() {
            match tokio::time::timeout(Duration::from_secs(5), guard.release()).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => self
                    .report
                    .log(LogKind::Warn, format!("project unlock: {err:#}")),
                Err(_) => self.report.log(LogKind::Warn, "project unlock timed out"),
            }
        }
    }

    fn check_project_lock(&self) -> Result<()> {
        if self
            .project_lock
            .as_ref()
            .is_some_and(|guard| !guard.is_alive())
        {
            bail!("the connection holding the project lock was lost");
        }
        Ok(())
    }

    fn at(&self, step: Step) -> usize {
        self.steps
            .iter()
            .position(|s| *s == step)
            .expect("the plan has every step the run reports")
    }

    fn start(&self, step: Step, what: &str) {
        self.report
            .start(self.at(step), &format!("{}: {what}", step.name()));
    }

    fn finish(&self, step: Step, detail: impl Into<String>) {
        self.report.finish(self.at(step), detail);
    }

    fn skip(&self, step: Step, detail: impl Into<String>) {
        self.report.skip(self.at(step), detail);
    }

    /// Marks every step after `failed` skipped, except Failed itself.
    fn skip_rest(&self, failed: Step, why: &str) {
        for (index, step) in self.steps.iter().enumerate().skip(self.at(failed) + 1) {
            if *step != Step::Failed {
                self.report.skip(index, why);
            }
        }
    }

    /// A failure before anything live changed.
    fn stop(&self, step: Step, err: &anyhow::Error) -> Ended {
        self.report.fail(self.at(step), short(err));
        self.report.log(LogKind::Error, format!("{err:#}"));
        self.skip_rest(step, "stopped");
        let error = format!("{err:#}");
        Ended {
            outcome: Outcome::Failed {
                error: error.clone(),
            },
            failed: Some((step, error)),
        }
    }

    /// `SHIPDECK_*` for a command in `step`.
    fn vars(&self, step: Step, error: Option<&str>) -> Vec<(&'static str, String)> {
        let config = self.config();
        let mut vars = vec![
            ("SHIPDECK_ACTION", self.action.key().to_string()),
            ("SHIPDECK_STEP", step.key().to_string()),
            ("SHIPDECK_NAME", config.name.clone()),
            ("SHIPDECK_SERVICE", config.service.name.clone()),
            ("SHIPDECK_HOST", self.deployable.endpoint.name.clone()),
            ("SHIPDECK_APP_DIR", config.service.dir.clone()),
            (
                "SHIPDECK_SHARED_DIR",
                format!("{}/shared", config.service.dir),
            ),
        ];
        if let Some(id) = &self.release {
            vars.push(("SHIPDECK_RELEASE", id.clone()));
        }
        if let Some(version) = &self.version {
            vars.push(("SHIPDECK_VERSION", version.clone()));
        }
        if let Some(previous) = &self.previous {
            vars.push(("SHIPDECK_PREVIOUS", previous.clone()));
        }
        if let Some(error) = error {
            vars.push(("SHIPDECK_ERROR", error.to_string()));
        }
        vars
    }

    /// Runs `commands` on this machine in `project`, one after another.
    /// Progress goes to `row`; `SHIPDECK_STEP` says `step`.
    async fn local(
        &self,
        row: Step,
        step: Step,
        error: Option<&str>,
        commands: &Commands,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }
        let dir = self.deployable.project_dir();
        if !dir.is_dir() {
            bail!("the project folder {} does not exist", dir.display());
        }
        let absolute = |path: &Path| {
            std::path::absolute(path)
                .unwrap_or(path.to_path_buf())
                .display()
                .to_string()
        };
        let mut env = self.vars(step, error);
        env.push(("SHIPDECK_FOLDER", absolute(&self.deployable.folder)));
        env.push(("SHIPDECK_PROJECT", absolute(&dir)));
        let index = self.at(row);
        for (done, command) in commands.iter().enumerate() {
            self.report.progress(
                index,
                done as u64,
                commands.len() as u64,
                Unit::Items,
                command.trim(),
            );
            Box::pin(build::run_local(command, &dir, &env, self.report, index)).await?;
        }
        Ok(())
    }

    /// Runs `commands` on the server at `place`, one after another.
    async fn remote(
        &self,
        server: &Server,
        place: Place,
        step: Step,
        commands: &Commands,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }
        let service = &self.config().service;
        let env = CommandEnv {
            service: service.env.clone(),
            env_file: service.env_file.as_deref().map(release_path),
            vars: self.vars(step, None),
        };
        let index = self.at(step);
        for (done, command) in commands.iter().enumerate() {
            self.report.progress(
                index,
                done as u64,
                commands.len() as u64,
                Unit::Items,
                command.trim(),
            );
            server.command(&place, command, &env, self.report).await?;
        }
        Ok(())
    }

    /// The deploy, step by step; `Ok` carries the release now live.
    async fn deploy(&mut self, pool: &Pool) -> Ended {
        match Box::pin(self.pipeline(pool)).await {
            Ok(id) => Ended::ok(&id),
            Err(ended) => ended,
        }
    }

    /// Records one step of this operation in the server's event database.
    async fn record(&self, server: &Server, kind: &str, detail: impl Into<String>) {
        let Some(agent) = server.session().agent() else {
            return;
        };
        let operation = self.release.clone().unwrap_or_default();
        if let Err(err) = agent
            .record_event(server.dir(), &operation, kind, &detail.into())
            .await
        {
            self.report
                .log(LogKind::Warn, format!("event {kind}: {err:#}"));
        }
    }

    /// Runs every step, stopping at the first failure.
    async fn pipeline(&mut self, pool: &Pool) -> Result<String, Ended> {
        let (server, facts) = Box::pin(self.step_preflight(pool)).await?;
        let result = Box::pin(self.after_preflight(&server, facts)).await;
        match &result {
            Ok(id) => {
                self.record(&server, "deploy_completed", format!("current → {id}"))
                    .await;
            }
            Err(ended) => {
                if let Outcome::Unchanged { live } = &ended.outcome {
                    self.record(&server, "deploy_unchanged", live.clone()).await;
                } else {
                    let why = ended
                        .failed
                        .as_ref()
                        .map(|(step, error)| format!("{}: {error}", step.key()))
                        .unwrap_or_default();
                    self.record(&server, "deploy_failed", why).await;
                }
            }
        }
        result
    }

    /// Every deploy step after preflight and its project lock.
    async fn after_preflight(&mut self, server: &Server, facts: Facts) -> Result<String, Ended> {
        let (name, id) = Box::pin(self.step_build(&facts)).await?;
        self.record(server, "build_completed", format!("releases/{id}"))
            .await;
        self.check_project_lock()
            .map_err(|err| self.stop(Step::Build, &err))?;
        let (manifest, contents, bundle) =
            Box::pin(self.step_pack(server, &name, &id, &facts)).await?;
        self.record(
            server,
            "pack_completed",
            format!("{id}: {}", bundle.data.len()),
        )
        .await;
        self.check_project_lock()
            .map_err(|err| self.stop(Step::Pack, &err))?;
        Box::pin(self.step_upload(server, &id, &bundle)).await?;
        self.check_project_lock()
            .map_err(|err| self.stop(Step::Upload, &err))?;
        Box::pin(self.step_unpack(server, &id, &contents, &manifest, &bundle)).await?;
        self.check_project_lock()
            .map_err(|err| self.stop(Step::Unpack, &err))?;
        Box::pin(self.step_setup(server, &id)).await?;
        self.record(server, "setup_completed", format!("releases/{id}"))
            .await;
        self.check_project_lock()
            .map_err(|err| self.stop(Step::Setup, &err))?;
        Box::pin(self.step_backup(server, &id, &bundle)).await?;
        self.record(server, "backup_completed", format!("backups/{id}.tar.zst"))
            .await;
        self.check_project_lock()
            .map_err(|err| self.stop(Step::Backup, &err))?;
        Box::pin(self.step_switch(server, &id)).await?;
        if let Err(err) = self.check_project_lock() {
            return Err(self.recover(server, Step::Switch, err).await);
        }
        Box::pin(self.step_restart(server)).await?;
        self.record(server, "service_restarted", "").await;
        if let Err(err) = self.check_project_lock() {
            return Err(self.recover(server, Step::Restart, err).await);
        }
        Box::pin(self.step_health(server)).await?;
        self.record(server, "health_passed", "").await;
        if let Err(err) = self.check_project_lock() {
            return Err(self.recover(server, Step::Health, err).await);
        }
        Box::pin(self.done(server)).await;
        if let Err(err) = self.check_project_lock() {
            return Err(self.connection_lost(Step::Done, &err));
        }
        Ok(id)
    }

    /// Asks whether to deploy a release `live` already holds. With nobody
    /// to answer, it does not.
    async fn deploy_unchanged(&self, live: &str) -> bool {
        let report = self.report;
        let id = self.release.clone().unwrap_or_default();
        report.log(
            LogKind::Info,
            format!(
                "nothing changed since {live}, the live release; asking whether to deploy anyway"
            ),
        );
        match report.ask_unchanged(&id, live).await {
            Ok(true) => {
                report.log(
                    LogKind::Info,
                    format!("deploying anyway, as a duplicate of {live}"),
                );
                true
            }
            Ok(false) | Err(_) => false,
        }
    }

    /// Stops a deploy whose release `live` already holds; nothing on the
    /// server was touched.
    fn unchanged(&self, step: Step, live: &str) -> Ended {
        self.report.skip(self.at(step), "nothing changed");
        self.skip_rest(step, "nothing changed");
        self.report.log(
            LogKind::Info,
            format!("nothing was deployed; {live} stays live"),
        );
        Ended {
            outcome: Outcome::Unchanged {
                live: live.to_string(),
            },
            failed: None,
        }
    }
}

#[cfg(test)]
#[path = "deploy_test.rs"]
mod tests;
