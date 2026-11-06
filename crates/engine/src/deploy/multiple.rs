//! One pipeline for apps that share a release and recovery boundary.

use std::sync::Arc;

use anyhow::{Result, bail};

use super::report::{LogKind, Outcome, Reporter};
use super::{Step, apps, preflight_apps};
use crate::config::monorepo::MonorepoConfig;
use crate::config::{MonorepoDeployment, Pipeline};
use crate::remote::Source;
use crate::ssh::Pool;

/// Every shared operation reports this fixed plan.
#[must_use]
pub fn monorepo_steps() -> Vec<Step> {
    vec![
        Step::Preflight,
        Step::Build,
        Step::Pack,
        Step::Upload,
        Step::Unpack,
        Step::Setup,
        Step::Backup,
        Step::Switch,
        Step::Restart,
        Step::Health,
        Step::Done,
        Step::Failed,
    ]
}

/// Files and live processes changed by this operation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReleaseState {
    Untouched,
    Staged,
    Restored,
    Switching,
}

/// State shared by a deployment and its failure recovery.
pub(super) struct Multiple<'a> {
    pub deployment: &'a MonorepoDeployment,
    pub config: MonorepoConfig,
    pub pipeline: Pipeline,
    pub report: &'a Reporter,
    pub release: Option<String>,
    pub previous: Option<String>,
    pub step: Step,
    pub rollback: bool,
    pub state: ReleaseState,
}

impl<'a> Multiple<'a> {
    pub fn new(
        deployment: &'a MonorepoDeployment,
        report: &'a Reporter,
        rollback: bool,
    ) -> Result<Self> {
        Ok(Self {
            deployment,
            config: deployment.config.clone(),
            pipeline: serde_json::from_value(deployment.config.pipeline.clone())?,
            report,
            release: None,
            previous: None,
            step: Step::Preflight,
            rollback,
            state: ReleaseState::Untouched,
        })
    }

    pub fn index(step: Step) -> usize {
        monorepo_steps()
            .iter()
            .position(|s| *s == step)
            .unwrap_or(0)
    }

    pub fn start(&mut self, step: Step) {
        self.step = step;
        self.report.start(Self::index(step), step.name());
    }

    pub fn finish(&self) {
        self.report.finish(Self::index(self.step), "");
    }

    /// Deploys all apps, preserving the old release until every health check passes.
    pub async fn deploy(&mut self, pool: &Pool) -> Result<Outcome> {
        self.start(Step::Preflight);
        let resolved = self.config.resolve_apps(&self.deployment.folder)?;
        let checked =
            preflight_apps(&resolved, &self.deployment.endpoint, pool, self.report).await?;
        let first = checked.first().ok_or_else(|| anyhow::anyhow!("no apps"))?;
        self.previous.clone_from(&first.facts.current);
        let session = pool.get(&self.deployment.endpoint).await?;
        self.check_permissions(&session).await?;
        self.hooks(&session, Step::Preflight, None).await?;
        self.finish();
        self.start(Step::Build);
        let root = self
            .deployment
            .folder
            .join(&self.config.project)
            .canonicalize()?;
        let version = self
            .pipeline
            .build
            .version_from
            .as_ref()
            .map(|file| root.join(file))
            .or_else(|| {
                crate::config::VERSION_FILES
                    .iter()
                    .map(|file| root.join(file))
                    .find(|p| p.is_file())
            });
        let (name, _) = crate::build::release_name_in(&root, version.as_deref()).await?;
        let id = name.id();
        self.release = Some(id.clone());
        self.report.release(&id);
        self.hooks(&session, Step::Build, None).await?;
        for checked in &checked {
            if let Some(binary) = &checked.app.pipeline.pack.binary {
                super::preflight::check_binary(
                    &checked.app.project.join(binary),
                    &checked.facts,
                    self.report,
                )?;
            }
        }
        self.finish();
        self.start(Step::Pack);
        let (contents, manifest, bundle) = self
            .bundle(&resolved, &name, &first.facts, &first.server)
            .await?;
        if self.previous.is_some() && first.facts.content == manifest.content {
            let live = self
                .previous
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no previous release"))?;
            if !self.report.ask_unchanged(&id, live).await.unwrap_or(false) {
                for step in monorepo_steps().into_iter().skip(Self::index(Step::Pack)) {
                    self.report.skip(Self::index(step), "nothing changed");
                }
                return Ok(Outcome::Unchanged {
                    live: live.to_string(),
                });
            }
        }
        self.finish();
        self.start(Step::Upload);
        if first
            .server
            .list()
            .await?
            .entries
            .iter()
            .any(|entry| entry.id == id)
        {
            bail!("release {id} already exists; wait one second before deploying again");
        }
        self.state = ReleaseState::Staged;
        self.send(&first.server, &id, &bundle).await?;
        self.finish();
        self.start(Step::Unpack);
        self.unpack(&first.server, &id, &contents, &manifest, &bundle)
            .await?;
        self.link(&first.server, &id).await?;
        self.finish();
        self.start(Step::Setup);
        self.hooks(&session, Step::Setup, None).await?;
        self.finish();
        self.start(Step::Backup);
        self.backup(&first.server, &id, &bundle.paths).await?;
        self.finish();
        self.activate(&session).await?;
        Ok(Outcome::Succeeded { release: id })
    }

    /// Restores a saved release, using its own apps and process managers.
    pub async fn rollback(&mut self, target: &str, pool: &Pool) -> Result<Outcome> {
        self.start(Step::Preflight);
        let session = pool.get(&self.deployment.endpoint).await?;
        let configured = apps::servers(&self.config, &session)?;
        let server = configured
            .first()
            .ok_or_else(|| anyhow::anyhow!("no apps"))?;
        let listing = server.list().await?;
        self.previous = listing.current;
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.id == target)
            .ok_or_else(|| anyhow::anyhow!("release {target} is not on the server"))?;
        if entry.current {
            bail!("{target} is already current");
        }
        let compressed = entry.kind == crate::remote::Kind::Compressed;
        let source = if compressed {
            Source::Backup
        } else {
            Source::Folder
        };
        self.config = apps::saved(server, target, source).await?;
        self.pipeline = serde_json::from_value(self.config.pipeline.clone())?;
        self.release = Some(target.to_string());
        self.report.release(target);
        self.check_permissions(&session).await?;
        self.finish();
        for step in [Step::Build, Step::Pack, Step::Upload] {
            self.report.skip(Self::index(step), "saved release");
        }
        self.start(Step::Unpack);
        if compressed {
            self.state = ReleaseState::Restored;
            server.restore(target, self.report).await?;
            self.link(server, target).await?;
        }
        self.finish();
        self.start(Step::Setup);
        if compressed {
            self.hooks(&session, Step::Setup, None).await?;
        }
        self.finish();
        self.report
            .skip(Self::index(Step::Backup), "existing backup");
        self.activate(&session).await?;
        Ok(Outcome::Succeeded {
            release: target.to_string(),
        })
    }

    /// Switches once, installs every process, restarts all apps, then checks them.
    async fn activate(&mut self, session: &Arc<crate::ssh::Session>) -> Result<()> {
        self.start(Step::Switch);
        self.check_permissions(session).await?;
        self.state = ReleaseState::Switching;
        self.transition(session).await?;
        self.finish();
        let servers = apps::servers(&self.config, session)?;
        self.start(Step::Restart);
        for server in &servers {
            server.restart(self.report).await?;
        }
        self.finish();
        self.start(Step::Health);
        self.check_health(session).await?;
        self.finish();
        self.start(Step::Done);
        let done = async {
            if !self.rollback {
                self.rotate(servers.first().ok_or_else(|| anyhow::anyhow!("no apps"))?)
                    .await?;
            }
            self.hooks(session, Step::Done, None).await
        }
        .await;
        match done {
            Ok(()) => self.finish(),
            Err(err) => {
                self.report.log(LogKind::Warn, format!("done: {err:#}"));
                self.report
                    .warn(Self::index(Step::Done), super::short(&err));
            }
        }
        Ok(())
    }
}

/// Runs a monorepo deployment and reports a single outcome.
pub async fn deploy_monorepo(deployment: MonorepoDeployment, pool: Arc<Pool>, report: Reporter) {
    execute(deployment, None, pool, report).await;
}

/// Rolls every app back to the settings saved in the target release.
pub async fn rollback_monorepo(
    deployment: MonorepoDeployment,
    target: String,
    pool: Arc<Pool>,
    report: Reporter,
) {
    execute(deployment, Some(target), pool, report).await;
}

async fn execute(
    deployment: MonorepoDeployment,
    target: Option<String>,
    pool: Arc<Pool>,
    report: Reporter,
) {
    let mut run = match Multiple::new(&deployment, &report, target.is_some()) {
        Ok(run) => run,
        Err(err) => {
            report.done(Outcome::Failed {
                error: format!("{err:#}"),
            });
            return;
        }
    };
    let result = match target {
        Some(target) => run.rollback(&target, &pool).await,
        None => run.deploy(&pool).await,
    };
    let outcome = match result {
        Ok(outcome) => {
            report.skip(Multiple::index(Step::Failed), "nothing failed");
            outcome
        }
        Err(err) => run.recover(&pool, err).await,
    };
    report.done(outcome);
}
