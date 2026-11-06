//! Global commands run once, followed by explicitly configured app commands.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};

use super::Step;
use super::multiple::Multiple;
use crate::config::monorepo::AppDefinition;
use crate::config::{Commands, Pipeline, Service};
use crate::remote::{CommandEnv, Place, Server};

impl Multiple<'_> {
    /// Runs global hooks first, then apps in name order.
    pub(super) async fn hooks(
        &self,
        session: &Arc<crate::ssh::Session>,
        step: Step,
        error: Option<&str>,
    ) -> Result<()> {
        let root = self.deployment.folder.join(&self.config.project);
        let servers = super::apps::servers(&self.config, session)?;
        let server = servers.first().ok_or_else(|| anyhow::anyhow!("no apps"))?;
        self.scope(Some(server), &self.pipeline, &root, None, step, error)
            .await?;
        for (name, definition) in &self.config.apps {
            let AppDefinition::Inline(app) = definition else {
                bail!("unresolved app {name}")
            };
            let own: Pipeline = serde_json::from_value(app.pipeline.clone())?;
            let service = app.service(&self.config, name);
            let server = Server::for_app(Arc::clone(session), &service, name)?;
            self.scope(
                Some(&server),
                &own,
                &root.join(&app.project),
                Some((name, &service)),
                step,
                error,
            )
            .await?;
        }
        Ok(())
    }

    /// Runs the commands supported by one pipeline step in their declared folders.
    async fn scope(
        &self,
        server: Option<&Server>,
        pipeline: &Pipeline,
        project: &Path,
        app: Option<(&str, &Service)>,
        step: Step,
        error: Option<&str>,
    ) -> Result<()> {
        let empty = Commands::default();
        let (remote, local, check) = match step {
            Step::Preflight => (
                &pipeline.preflight.run.server,
                &pipeline.preflight.run.local,
                &empty,
            ),
            Step::Build => (&empty, &pipeline.build.run.local, &empty),
            Step::Setup => (
                &pipeline.setup.run.server,
                &empty,
                &pipeline.setup.check.server,
            ),
            Step::Health => (
                &pipeline.health.check.server,
                &pipeline.health.check.local,
                &empty,
            ),
            Step::Done => (&pipeline.done.run.server, &pipeline.done.run.local, &empty),
            Step::Failed => (&empty, &pipeline.failed.run.local, &empty),
            _ => (&empty, &empty, &empty),
        };
        let mut env = self.environment(project, app, step, error)?;
        let place = match (step, app, self.release.as_ref()) {
            (Step::Preflight, _, _) => {
                if let Some((name, _)) = app {
                    env.env_file = env.env_file.map(|f| format!("apps/{name}/{f}"));
                }
                Place::AppDir
            }
            (Step::Setup, Some((name, _)), Some(id)) => {
                Place::AppRelease(id.clone(), name.to_string())
            }
            (Step::Setup, None, Some(id)) => Place::Release(id.clone()),
            (_, Some((name, _)), _) => Place::AppCurrent(name.to_string()),
            _ => Place::Current,
        };
        for command in remote.iter().chain(check.iter()) {
            server
                .ok_or_else(|| anyhow::anyhow!("server is unavailable"))?
                .command(&place, command, &env, self.report)
                .await?;
        }
        for command in local.iter() {
            Box::pin(crate::build::run_local(
                command,
                project,
                &env.vars,
                self.report,
                Self::index(step),
            ))
            .await?;
        }
        Ok(())
    }

    /// Builds the documented environment for one command scope.
    fn environment(
        &self,
        project: &Path,
        app: Option<(&str, &Service)>,
        step: Step,
        error: Option<&str>,
    ) -> Result<CommandEnv> {
        let mut vars = vec![
            (
                "SHIPDECK_ACTION",
                if self.rollback { "rollback" } else { "deploy" }.to_string(),
            ),
            (
                "SHIPDECK_STEP",
                if step == Step::Failed {
                    self.step
                } else {
                    step
                }
                .key()
                .to_string(),
            ),
            ("SHIPDECK_NAME", self.config.name.clone()),
            ("SHIPDECK_HOST", self.deployment.endpoint.name.clone()),
            ("SHIPDECK_APP_DIR", self.config.dir.clone()),
            (
                "SHIPDECK_FOLDER",
                std::path::absolute(&self.deployment.folder)?
                    .display()
                    .to_string(),
            ),
            (
                "SHIPDECK_PROJECT",
                std::path::absolute(project)?.display().to_string(),
            ),
            ("SHIPDECK_SHARED_DIR", format!("{}/shared", self.config.dir)),
        ];
        if let Some(id) = &self.release {
            vars.push(("SHIPDECK_RELEASE", id.clone()));
            if let Some(name) = crate::release::ReleaseName::parse(id) {
                vars.push(("SHIPDECK_VERSION", name.version));
            }
        }
        if let Some(previous) = &self.previous {
            vars.push(("SHIPDECK_PREVIOUS", previous.clone()));
        }
        if let Some(error) = error {
            vars.push(("SHIPDECK_ERROR", error.to_string()));
        }
        let mut env = CommandEnv {
            vars,
            ..CommandEnv::default()
        };
        if let Some((name, service)) = app {
            env.service.clone_from(&service.env);
            env.env_file.clone_from(&service.env_file);
            env.vars.push(("SHIPDECK_APP", name.to_string()));
            env.vars.push(("SHIPDECK_SERVICE", service.name.clone()));
        }
        Ok(env)
    }

    /// Runs failure commands locally even when the SSH connection failed.
    pub(super) async fn failed_hooks(&self, error: &str) -> Result<()> {
        let root = self.deployment.folder.join(&self.config.project);
        self.scope(None, &self.pipeline, &root, None, Step::Failed, Some(error))
            .await?;
        for (name, definition) in &self.config.apps {
            let AppDefinition::Inline(app) = definition else {
                bail!("unresolved app {name}")
            };
            let own: Pipeline = serde_json::from_value(app.pipeline.clone())?;
            let service = app.service(&self.config, name);
            self.scope(
                None,
                &own,
                &root.join(&app.project),
                Some((name, &service)),
                Step::Failed,
                Some(error),
            )
            .await?;
        }
        Ok(())
    }

    /// Checks every app, then retries command checks within the shared timeout.
    pub(super) async fn check_health(&self, session: &Arc<crate::ssh::Session>) -> Result<()> {
        let servers = super::apps::servers(&self.config, session)?;
        for ((name, definition), server) in self.config.apps.iter().zip(&servers) {
            let AppDefinition::Inline(app) = definition else {
                bail!("unresolved app {name}")
            };
            let pipeline = app.pipeline(&self.config.pipeline)?;
            super::health::wait_healthy(
                server,
                &pipeline.health,
                self.report,
                Self::index(Step::Health),
            )
            .await?;
        }
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(self.pipeline.health.timeout_secs);
        loop {
            match self.hooks(session, Step::Health, None).await {
                Ok(()) => return Ok(()),
                Err(err) if tokio::time::Instant::now() >= deadline => return Err(err),
                Err(err) => self.report.log(
                    super::report::LogKind::Warn,
                    format!("health check: {err:#}; retrying"),
                ),
            }
            tokio::time::sleep(super::health::CHECK_RETRY).await;
        }
    }
}

#[cfg(test)]
#[path = "multiple_hooks_tests.rs"]
mod tests;
