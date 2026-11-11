//! One deploy or rollback in progress: its plan, what is known so far, and
//! the report it writes to. The step bodies live in the modules beside this.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use super::commands::Exec;
use super::report::{LogKind, Outcome, Reporter};
use super::steps::{Action, Step, deploy_steps, rollback_steps};
use crate::bundle::Plan;
use crate::config::Deployment;
use crate::ssh::{Pool, Session};

/// A run, from its first step to its last.
pub(crate) struct Run<'a> {
    pub deployment: &'a Deployment,
    pub report: &'a Reporter,
    pub steps: Vec<Step>,
    pub action: Action,
    pub release: Option<String>,
    pub version: Option<String>,
    pub previous: Option<String>,
    pub failed_step: Option<Step>,
    pub project_lock: Option<crate::agent::ProjectGuard>,
    pub session: Option<Arc<Session>>,
    pub plan: Option<Plan>,
    /// Where preflight found each app's start program.
    pub start_paths: BTreeMap<String, String>,
    /// A rollback target is read from its backup.
    pub from_backup: bool,
}

/// How a run ended, and the step that failed if one did.
pub(crate) struct Ended {
    pub outcome: Outcome,
    pub failed: Option<(Step, String)>,
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

impl<'a> Run<'a> {
    pub(crate) fn new(
        deployment: &'a Deployment,
        report: &'a Reporter,
        steps: Vec<Step>,
        action: Action,
    ) -> Self {
        Self {
            deployment,
            report,
            steps,
            action,
            release: None,
            version: None,
            previous: None,
            failed_step: None,
            project_lock: None,
            session: None,
            plan: None,
            start_paths: BTreeMap::new(),
            from_backup: false,
        }
    }

    /// The row of a step in the plan.
    pub(crate) fn at(&self, step: Step) -> usize {
        self.steps
            .iter()
            .position(|plan| *plan == step)
            .unwrap_or_default()
    }

    pub(crate) fn start(&self, step: Step, what: &str) {
        self.report
            .start(self.at(step), &format!("{}: {what}", step.name()));
    }

    pub(crate) fn finish(&self, step: Step, detail: impl Into<String>) {
        self.report.finish(self.at(step), detail);
    }

    pub(crate) fn skip(&self, step: Step, detail: impl Into<String>) {
        let detail = detail.into();
        self.report.skip(
            self.at(step),
            if detail.is_empty() {
                "nothing to do".to_string()
            } else {
                detail
            },
        );
    }

    pub(crate) fn warn(&self, step: Step, detail: impl Into<String>) {
        self.report.warn(self.at(step), detail);
    }

    /// Marks every step after `failed` skipped, except Failed.
    pub(crate) fn skip_rest(&self, failed: Step, why: &str) {
        for (index, step) in self.steps.iter().enumerate().skip(self.at(failed) + 1) {
            if *step != Step::Failed {
                self.report.skip(index, why);
            }
        }
    }

    /// The command runner for one step.
    pub(crate) fn exec(&self, step: Step) -> Exec<'a> {
        Exec {
            deployment: self.deployment,
            report: self.report,
            session: self.session.clone(),
            release: self.release.clone(),
            version: self.version.clone(),
            previous: self.previous.clone(),
            failed_step: self.failed_step,
            action: self.action,
            row: self.at(step),
        }
    }

    /// A failure before anything live changed.
    pub(crate) fn stop(&mut self, step: Step, err: &anyhow::Error) -> Ended {
        self.failed_step = Some(step);
        self.report.fail(self.at(step), super::short(err));
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

    /// Checks the project lock is still held.
    ///
    /// # Errors
    /// Returns an error when the connection that holds the lock ended.
    pub(crate) fn check_lock(&self) -> Result<()> {
        if self
            .project_lock
            .as_ref()
            .is_some_and(|guard| !guard.is_alive())
        {
            bail!("the connection holding the project lock was lost");
        }
        Ok(())
    }

    /// Marks a step failed without stopping the run.
    pub(crate) fn fail(&self, step: Step, detail: impl Into<String>) {
        self.report.fail(self.at(step), detail);
    }

    /// Releases the project lock, with a warning when that fails.
    pub(crate) async fn unlock(&mut self) {
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

    /// Records one step in the server's event database.
    pub(crate) async fn record(&self, kind: &str, detail: impl Into<String>) {
        let Some(session) = &self.session else {
            return;
        };
        let Some(agent) = session.agent() else {
            return;
        };
        let operation = self.release.clone().unwrap_or_default();
        if let Err(err) = agent
            .record_event(self.deployment.dir(), &operation, kind, &detail.into())
            .await
        {
            self.report
                .log(LogKind::Warn, format!("event {kind}: {err:#}"));
        }
    }

    /// Runs the deploy. The failed step always runs last.
    pub(crate) async fn deploy(&mut self, pool: &Pool) -> Ended {
        let ended = match self.pipeline(pool).await {
            Ok(id) => Ended::ok(&id),
            Err(ended) => ended,
        };
        Box::pin(self.failed(&ended)).await;
        ended
    }

    /// Runs every deploy step, stopping at the first failure.
    async fn pipeline(&mut self, pool: &Pool) -> Result<String, Ended> {
        let (server, facts) = Box::pin(self.step_preflight(pool)).await?;
        let (_, id) = Box::pin(self.step_build(&facts)).await?;
        Box::pin(self.step_upload(&server, &facts)).await?;
        Box::pin(self.step_setup(&server)).await?;
        Box::pin(self.step_backup(&server)).await?;
        Box::pin(self.step_switch(&server)).await?;
        Box::pin(self.step_restart(&server)).await?;
        Box::pin(self.step_nginx(&server)).await?;
        Box::pin(self.step_health(&server)).await?;
        Box::pin(self.step_done(&server)).await?;
        self.record("deploy_completed", format!("current -> {id}"))
            .await;
        Ok(id)
    }

    /// Runs a rollback by hand. The failed step runs only if it fails.
    pub(crate) async fn rollback(&mut self, pool: &Pool, target: &str) -> Ended {
        match self.rollback_pipeline(pool, target).await {
            Ok(()) => Ended::ok(target),
            Err(ended) => {
                Box::pin(self.failed(&ended)).await;
                ended
            }
        }
    }

    /// The rollback steps, with the target's own settings.
    async fn rollback_pipeline(&mut self, pool: &Pool, target: &str) -> Result<(), Ended> {
        Box::pin(self.rollback_preflight(pool, target)).await?;
        if let Err(err) = Box::pin(self.step_put_back(target)).await {
            return Err(self.stop_rollback(Step::Rollback, &err));
        }
        Ok(())
    }
}

/// The steps a deploy plan has.
#[must_use]
pub fn deploy_plan() -> Vec<Step> {
    deploy_steps()
}

/// The steps a rollback plan has.
#[must_use]
pub fn rollback_plan() -> Vec<Step> {
    rollback_steps()
}

/// Runs a deploy and reports a single outcome.
pub async fn deploy(deployment: Deployment, pool: Arc<Pool>, report: Reporter) {
    let mut run = Run::new(&deployment, &report, deploy_plan(), Action::Deploy);
    let ended = Box::pin(run.deploy(&pool)).await;
    run.unlock().await;
    report.done(ended.outcome);
}

/// Runs a rollback by hand and reports a single outcome.
pub async fn rollback(deployment: Deployment, target: String, pool: Arc<Pool>, report: Reporter) {
    report.release(&target);
    let mut run = Run::new(&deployment, &report, rollback_plan(), Action::Rollback);
    run.release = Some(target.clone());
    run.version = crate::release::ReleaseName::parse(&target).map(|name| name.version);
    let ended = Box::pin(run.rollback(&pool, &target)).await;
    run.unlock().await;
    report.done(ended.outcome);
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
