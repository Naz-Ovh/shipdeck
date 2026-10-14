//! Manual rollback to any release on the server, open or compressed.

use std::sync::Arc;

use anyhow::{Result, bail};

use super::report::{LogKind, Outcome, Reporter};
use super::{recover, restore, short, skip_rest, wait_healthy};
use crate::config::Deployable;
use crate::remote::{Kind, Server};
use crate::ssh::Pool;

pub const ROLLBACK_STEPS: [&str; 5] = ["Connect", "Unpack", "Switch", "Restart", "Health"];
const CONNECT: usize = 0;
const UNPACK: usize = 1;
const SWITCH: usize = 2;
const RESTART: usize = 3;
const HEALTH: usize = 4;

pub async fn rollback(deployable: Deployable, target: String, pool: Arc<Pool>, report: Reporter) {
    report.release(&target);
    let outcome = run(&deployable, &target, &pool, &report).await;
    report.done(outcome);
}

async fn run(deployable: &Deployable, target: &str, pool: &Pool, report: &Reporter) -> Outcome {
    let config = &deployable.config;

    report.start(CONNECT, "Connect: checking the server");
    let (server, kind, previous) = match connect(deployable, target, pool, report).await {
        Ok(found) => found,
        Err(err) => return stop(report, CONNECT, err),
    };

    report.start(UNPACK, "Unpack: making the release ready");
    if kind == Kind::Compressed {
        if let Err(err) = server.restore(target, report).await {
            return stop(report, UNPACK, err);
        }
        report.finish(UNPACK, "unpacked from backups/");
    } else {
        report.skip(UNPACK, "already unpacked");
    }

    report.start(SWITCH, "Switch: making it the current release");
    if let Err(err) = restore(&server, target, report).await {
        report.fail(SWITCH, short(&err));
        report.log(LogKind::Error, format!("{err:#}"));
        skip_rest(report, SWITCH, ROLLBACK_STEPS.len(), "stopped");
        if let Some(previous) = &previous
            && let Err(restore_err) = restore(&server, previous, report).await
        {
            return Outcome::RollbackFailed {
                error: format!("{err:#}"),
                rollback_error: format!("{restore_err:#}"),
            };
        }
        return Outcome::Failed {
            error: format!("{err:#}"),
        };
    }
    report.finish(SWITCH, format!("current → {target}"));

    report.start(RESTART, &format!("Restart: {}", config.remote.service));
    if let Err(err) = server.restart(report).await {
        return recover(
            &server,
            report,
            RESTART,
            err,
            previous.as_deref(),
            None,
            ROLLBACK_STEPS.len(),
        )
        .await;
    }
    report.finish(RESTART, "");

    report.start(HEALTH, "Health: waiting for the release to settle");
    match wait_healthy(&server, &config.health, report, HEALTH).await {
        Ok(summary) => report.finish(HEALTH, summary),
        Err(err) => {
            return recover(
                &server,
                report,
                HEALTH,
                err,
                previous.as_deref(),
                None,
                ROLLBACK_STEPS.len(),
            )
            .await;
        }
    }

    report.log(LogKind::Success, format!("{target} is live"));
    Outcome::Succeeded {
        release: target.to_string(),
    }
}

/// Connects, checks sudo, and finds the target and the live release.
async fn connect(
    deployable: &Deployable,
    target: &str,
    pool: &Pool,
    report: &Reporter,
) -> Result<(Server, Kind, Option<String>)> {
    let config = &deployable.config;
    let endpoint = &deployable.endpoint;
    report.log(LogKind::Command, format!("connect {}", endpoint.label()));
    let session = pool.get(endpoint).await?;
    let server = Server::new(session, &config.remote);
    let facts = server.facts(&endpoint.user, &[]).await?;
    if !facts.sudo_missing.is_empty() {
        super::setup_hint(
            deployable,
            pool.login(),
            report,
            "the server needs its one-time setup",
        );
        bail!("{} may not run the commands shipdeck needs", endpoint.user);
    }
    let listing = server.list().await?;
    let Some(entry) = listing.entries.iter().find(|e| e.id == target) else {
        bail!("{target} is no longer on the server");
    };
    if entry.current {
        bail!("{target} is already the current release");
    }
    report.finish(
        CONNECT,
        format!("live: {}", listing.current.as_deref().unwrap_or("nothing")),
    );
    Ok((server, entry.kind, listing.current.clone()))
}

fn stop(report: &Reporter, step: usize, err: anyhow::Error) -> Outcome {
    report.fail(step, short(&err));
    report.log(LogKind::Error, format!("{err:#}"));
    skip_rest(report, step, ROLLBACK_STEPS.len(), "stopped");
    Outcome::Failed {
        error: format!("{err:#}"),
    }
}
