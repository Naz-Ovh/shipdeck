//! Failure cleanup and restoration of the complete previous app set.

use std::sync::Arc;

use anyhow::Result;

use super::multiple::{Multiple, ReleaseState};
use super::report::{LogKind, Outcome};
use super::{Step, apps};
use crate::config::{ROLLBACK_ANSWER_SECS, Rollback};
use crate::remote::{Server, Source};

impl Multiple<'_> {
    /// Restores the previous settings after a partial switch or health failure.
    pub(super) async fn recover(&mut self, pool: &crate::ssh::Pool, err: anyhow::Error) -> Outcome {
        let error = format!("{err:#}");
        let failed = self.step;
        self.report.fail(Self::index(failed), super::short(&err));
        self.report.log(LogKind::Error, &error);
        for step in super::monorepo_steps()
            .into_iter()
            .skip(Self::index(failed) + 1)
            .filter(|s| *s != Step::Failed)
        {
            self.report.skip(Self::index(step), "stopped");
        }
        let outcome = match pool.get(&self.deployment.endpoint).await {
            Ok(session) if self.state == ReleaseState::Switching => {
                self.recover_live(&session, failed, &error).await
            }
            Ok(session) => {
                if let Err(err) = self.cleanup(&session).await {
                    self.report.log(LogKind::Warn, format!("cleanup: {err:#}"));
                }
                Outcome::Failed {
                    error: error.clone(),
                }
            }
            Err(err) if self.state == ReleaseState::Switching => Outcome::RollbackFailed {
                error: error.clone(),
                rollback_error: format!("reconnecting for recovery: {err:#}"),
            },
            Err(_) => Outcome::Failed {
                error: error.clone(),
            },
        };
        self.report
            .start(Self::index(Step::Failed), "running failed commands");
        match self.failed_hooks(&error).await {
            Ok(()) => self.report.finish(Self::index(Step::Failed), ""),
            Err(err) => self
                .report
                .warn(Self::index(Step::Failed), super::short(&err)),
        }
        outcome
    }

    /// Chooses recovery once for the entire release.
    async fn recover_live(
        &self,
        session: &Arc<crate::ssh::Session>,
        failed: Step,
        error: &str,
    ) -> Outcome {
        let Some(previous) = &self.previous else {
            return Outcome::Kept {
                release: self.release.clone().unwrap_or_default(),
                error: error.to_string(),
            };
        };
        let rollback = self.pipeline.failed.rollback == Rollback::Auto
            || tokio::time::timeout(
                std::time::Duration::from_secs(ROLLBACK_ANSWER_SECS),
                self.report.ask_rollback(
                    self.release.as_deref().unwrap_or_default(),
                    previous,
                    failed.name().to_string(),
                    error,
                    ROLLBACK_ANSWER_SECS,
                ),
            )
            .await
            .map_or(true, |answer| answer.unwrap_or(true));
        if !rollback {
            return Outcome::Kept {
                release: self.release.clone().unwrap_or_default(),
                error: error.to_string(),
            };
        }
        let restored = async {
            let servers = apps::servers(&self.config, session)?;
            self.restore_previous(session, &servers, previous).await
        }
        .await;
        match restored {
            Ok(()) => Outcome::RolledBack {
                error: error.to_string(),
                to: previous.clone(),
            },
            Err(err) => Outcome::RollbackFailed {
                error: error.to_string(),
                rollback_error: format!("{err:#}"),
            },
        }
    }

    /// Removes only files created by this operation before a switch.
    async fn cleanup(&self, session: &Arc<crate::ssh::Session>) -> Result<()> {
        let servers = apps::servers(&self.config, session)?;
        if let (Some(server), Some(id)) = (servers.first(), &self.release) {
            if !self.rollback && self.state == ReleaseState::Staged {
                server.discard(id, self.report).await?;
            } else if self.rollback && self.state == ReleaseState::Restored {
                server.drop_open(id, self.report).await?;
            }
        }
        Ok(())
    }

    /// Retires new processes, restores saved settings, and checks all restored apps.
    async fn restore_previous(
        &self,
        session: &Arc<crate::ssh::Session>,
        servers: &[Server],
        previous: &str,
    ) -> Result<()> {
        let server = servers.first().ok_or_else(|| anyhow::anyhow!("no apps"))?;
        let config = apps::saved(server, previous, Source::Folder).await?;
        let mut restore = Self::new(self.deployment, self.report, true)?;
        restore.config = config;
        restore.pipeline = serde_json::from_value(restore.config.pipeline.clone())?;
        restore.release = Some(previous.to_string());
        restore.previous.clone_from(&self.release);
        restore.check_permissions(session).await?;
        restore.transition(session).await?;
        for server in apps::servers(&restore.config, session)? {
            server.restart(self.report).await?;
        }
        restore.check_health(session).await?;
        if !self.rollback
            && let Some(id) = &self.release
        {
            server.discard(id, self.report).await?;
        }
        Ok(())
    }
}
