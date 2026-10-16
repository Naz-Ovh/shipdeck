//! When a step fails: the failed commands and the question whether to roll back.

use std::time::Duration;

use anyhow::bail;

use super::report::{LogKind, Outcome};
use super::{Action, Ended, Run, Step, backup::restore, count, short};
use crate::config::{ROLLBACK_ANSWER_SECS, Rollback};
use crate::remote::Server;

impl<'a> Run<'a> {
    /// Runs `pipeline.failed` when a step failed. Its own failure is only
    /// logged.
    pub(crate) async fn failed(&self, ended: &Ended) {
        if !self.steps.contains(&Step::Failed) {
            return;
        }
        let Some((step, error)) = &ended.failed else {
            self.skip(Step::Failed, "nothing failed");
            return;
        };
        self.start(
            Step::Failed,
            &format!("running pipeline.failed (after {})", step.key()),
        );
        let first = error.lines().next().unwrap_or_default();
        let commands = &self.config().pipeline.failed.run.local;
        match self.local(Step::Failed, *step, Some(first), commands).await {
            Ok(()) => self.finish(Step::Failed, count(commands.len(), "command")),
            Err(err) => {
                self.report
                    .log(LogKind::Warn, format!("pipeline.failed: {err:#}"));
                self.report.warn(self.at(Step::Failed), short(&err));
            }
        }
    }

    /// After a failed switch, restart or health check: show the journal,
    /// ask (or not) and put the previous release back.
    pub(super) async fn recover(&self, server: &Server, step: Step, err: anyhow::Error) -> Ended {
        let report = self.report;
        report.fail(self.at(step), short(&err));
        report.log(LogKind::Error, format!("{err:#}"));
        match server.journal().await {
            Ok(lines) if !lines.is_empty() => {
                report.log(LogKind::Info, "last log lines:");
                for line in lines {
                    report.log(LogKind::Err, line);
                }
            }
            _ => report.log(LogKind::Warn, "could not read the service's log"),
        }
        let error = format!("{err:#}");
        let id = self.release.clone().unwrap_or_default();
        let manager = server.manager().label();

        let Some(previous) = self.previous.clone().filter(|p| *p != id) else {
            report.log(
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
            report.log(
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
        report.log(LogKind::Warn, format!("rolling back to {previous}"));
        let restored = async {
            restore(server, &previous, report).await?;
            server.restart(report).await?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let probe = server.probe(None).await?;
            if probe.state != "active" {
                bail!(
                    "{previous} did not come back either: {manager} says {}",
                    probe.state
                );
            }
            Ok(())
        }
        .await;
        let outcome = match restored {
            Ok(()) => {
                // A deploy's failed release goes; a rollback's target was an
                // old release and stays.
                if self.action == Action::Deploy {
                    let _ = server.discard(&id, report).await;
                }
                report.log(LogKind::Success, format!("{previous} is live again"));
                Outcome::RolledBack {
                    error: error.clone(),
                    to: previous,
                }
            }
            Err(rollback_err) => {
                report.log(LogKind::Error, format!("rollback failed: {rollback_err:#}"));
                Outcome::RollbackFailed {
                    error: error.clone(),
                    rollback_error: format!("{rollback_err:#}"),
                }
            }
        };
        Ended {
            outcome,
            failed: Some((step, error)),
        }
    }

    /// `pipeline.failed.rollback`: auto rolls back; ask waits for an answer
    /// and rolls back when none comes in time.
    pub(super) async fn should_roll_back(
        &self,
        step: Step,
        err: &anyhow::Error,
        previous: &str,
    ) -> bool {
        let report = self.report;
        if self.config().pipeline.failed.rollback == Rollback::Auto {
            return true;
        }
        let id = self.release.clone().unwrap_or_default();
        report.log(
            LogKind::Info,
            format!(
                "asking whether to roll back to {previous}; with no answer in {ROLLBACK_ANSWER_SECS}s it rolls back"
            ),
        );
        let answer = report.ask_rollback(
            &id,
            previous,
            step.name(),
            &short(err),
            ROLLBACK_ANSWER_SECS,
        );
        match tokio::time::timeout(Duration::from_secs(ROLLBACK_ANSWER_SECS), answer).await {
            Ok(Ok(true)) => {
                report.log(LogKind::Info, "you chose to roll back");
                true
            }
            Ok(Ok(false)) => {
                report.log(LogKind::Warn, format!("you chose to keep {id}"));
                false
            }
            // Nobody is there to answer.
            Ok(Err(_)) => true,
            Err(_) => {
                report.log(
                    LogKind::Warn,
                    format!("no answer in {ROLLBACK_ANSWER_SECS}s; rolling back"),
                );
                true
            }
        }
    }
}
