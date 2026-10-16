//! The health checks after the restart, and the notes once a release is live.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use super::report::{LogKind, Reporter, Unit};
use super::{Action, Run, Step, backup::rotate, count, short};
use crate::human;
use crate::remote::{HttpCheck, Place, Server};

/// Consecutive healthy probes, one second apart, before a release counts as up.
pub(super) const STABLE_PROBES: u32 = 3;
/// The pause before a failed health check runs again.
pub(super) const CHECK_RETRY: Duration = Duration::from_secs(2);

impl<'a> Run<'a> {
    /// Waits until the service is up, then runs the health checks, trying
    /// them again until `timeout_secs` has passed.
    pub(super) async fn health(&self, server: &Server) -> Result<String> {
        let health = &self.config().pipeline.health;
        let index = self.at(Step::Health);
        let started = Instant::now();
        let summary = wait_healthy(server, health, self.report, index).await?;
        let checks = &health.check;
        if checks.server.is_empty() && checks.local.is_empty() {
            return Ok(summary);
        }
        let deadline = started + Duration::from_secs(health.timeout_secs);
        loop {
            let mut passed = self
                .remote(server, Place::Current, Step::Health, &checks.server)
                .await;
            if passed.is_ok() {
                passed = self
                    .local(Step::Health, Step::Health, None, &checks.local)
                    .await;
            }
            match passed {
                Ok(()) => {
                    return Ok(format!(
                        "{summary} · {} passed",
                        count(checks.server.len() + checks.local.len(), "check")
                    ));
                }
                Err(err) if Instant::now() + CHECK_RETRY < deadline => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    self.report.log(
                        LogKind::Warn,
                        format!(
                            "{err:#}; trying again in {}s ({}s left)",
                            CHECK_RETRY.as_secs(),
                            left.as_secs()
                        ),
                    );
                    tokio::time::sleep(CHECK_RETRY).await;
                }
                Err(err) => {
                    return Err(err.context(format!(
                        "the health checks still fail after {}s",
                        health.timeout_secs
                    )));
                }
            }
        }
    }

    /// The release is live: tidy old releases after a deploy, then run
    /// `pipeline.done`. Nothing here changes what is live.
    pub(super) async fn done(&self, server: &Server) {
        let report = self.report;
        let id = self.release.clone().unwrap_or_default();
        report.log(LogKind::Success, format!("{id} is live"));
        let run = &self.config().pipeline.done.run;
        if self.action == Action::Rollback && run.server.is_empty() && run.local.is_empty() {
            self.skip(Step::Done, "nothing to run");
            return;
        }
        self.start(
            Step::Done,
            if self.action == Action::Deploy {
                "tidying old releases"
            } else {
                "running pipeline.done"
            },
        );
        let index = self.at(Step::Done);
        let mut notes = Vec::new();
        let mut problem = None;
        if self.action == Action::Deploy {
            match rotate(server, self.deployable, report, index).await {
                Ok(summary) => notes.push(summary),
                Err(err) => {
                    report.log(LogKind::Warn, format!("tidying: {err:#}"));
                    problem = Some(err);
                }
            }
        }
        let mut ran = self
            .remote(server, Place::Current, Step::Done, &run.server)
            .await;
        if ran.is_ok() {
            ran = self.local(Step::Done, Step::Done, None, &run.local).await;
        }
        match ran {
            Ok(()) if !(run.server.is_empty() && run.local.is_empty()) => {
                notes.push(count(run.server.len() + run.local.len(), "command"));
            }
            Ok(()) => {}
            Err(err) => {
                report.log(
                    LogKind::Warn,
                    format!("pipeline.done: {err:#}; {id} stays live"),
                );
                problem = Some(err);
            }
        }
        match problem {
            Some(err) => report.warn(index, short(&err)),
            None => self.finish(Step::Done, notes.join(" · ")),
        }
    }
}

/// Probes once a second until the service has been healthy `STABLE_PROBES`
/// times in a row, it crash-loops, or the timeout passes.
pub(crate) async fn wait_healthy(
    server: &Server,
    health: &crate::config::Health,
    report: &Reporter,
    step: usize,
) -> Result<String> {
    let timeout = Duration::from_secs(health.timeout_secs);
    let started = Instant::now();
    let manager = server.manager().label();
    let mut baseline = None;
    let mut streak = 0;
    loop {
        let probe = server.probe(health.url.as_deref()).await?;
        if baseline.is_none() {
            baseline = probe.restarts;
        }
        if let (Some(before), Some(now)) = (baseline, probe.restarts)
            && now > before
        {
            bail!(
                "the service crashed and was restarted {} time(s) by {manager}",
                now - before
            );
        }
        if probe.state == "failed" {
            bail!("{manager} reports the service as failed");
        }
        if probe.http == HttpCheck::NoClient {
            bail!("pipeline.health.url is set, but the server has neither curl nor wget");
        }
        streak = if probe.healthy() { streak + 1 } else { 0 };

        let http = match &probe.http {
            HttpCheck::Skipped => String::new(),
            HttpCheck::Ok => " · http ok".into(),
            HttpCheck::Failed(why) => format!(" · http: {why}"),
            HttpCheck::NoClient => String::new(),
        };
        let elapsed = started.elapsed();
        report.progress(
            step,
            elapsed.as_secs().min(health.timeout_secs),
            health.timeout_secs,
            Unit::Seconds,
            format!("{}{http} · stable {streak}/{STABLE_PROBES}", probe.state),
        );

        if streak >= STABLE_PROBES {
            let checked = if health.url.is_some() {
                "active, url answers"
            } else {
                "active"
            };
            return Ok(format!("{checked} · {}", human::duration(elapsed)));
        }
        if elapsed >= timeout {
            let why = match probe.http {
                HttpCheck::Failed(why) => format!("; the health url said: {why}"),
                _ => String::new(),
            };
            bail!(
                "not healthy after {}s: {manager} says {}{why}",
                health.timeout_secs,
                if probe.state.is_empty() {
                    "nothing"
                } else {
                    &probe.state
                }
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
