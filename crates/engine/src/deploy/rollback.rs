//! Manual rollback to any release on the server, open or compressed. A
//! backup is unpacked and set up again with `pipeline.setup`. It ends like a
//! deploy: health, then `pipeline.done`, all with SHIPDECK_ACTION=rollback.

use std::sync::Arc;

use anyhow::{Result, bail};

use super::report::{LogKind, Reporter};
use super::{Action, Ended, Run, Step, restore};
use crate::config::{DeployConfig, Deployable};
use crate::release::ReleaseName;
use crate::remote::{Kind, ReleaseEntry, Server};
use crate::ssh::Pool;

/// The steps a rollback goes through.
pub fn rollback_steps(config: &DeployConfig) -> Vec<Step> {
    let mut steps = vec![
        Step::Connect,
        Step::Unpack,
        Step::Setup,
        Step::Switch,
        Step::Restart,
        Step::Health,
        Step::Done,
    ];
    if !config.pipeline.failed.run.local.is_empty() {
        steps.push(Step::Failed);
    }
    steps
}

/// The names of [`rollback_steps`], for the screen.
pub fn rollback_step_names(config: &DeployConfig) -> Vec<&'static str> {
    rollback_steps(config).into_iter().map(Step::name).collect()
}

pub async fn rollback(deployable: Deployable, target: String, pool: Arc<Pool>, report: Reporter) {
    report.release(&target);
    let mut run = Run::new(
        &deployable,
        &report,
        rollback_steps(&deployable.config),
        Action::Rollback,
    );
    run.release = Some(target.clone());
    run.version = ReleaseName::parse(&target).map(|name| name.version);
    let ended = run.rollback(&target, &pool).await;
    run.failed(&ended).await;
    report.done(ended.outcome);
}

impl Run<'_> {
    async fn rollback(&mut self, target: &str, pool: &Pool) -> Ended {
        let report = self.report;

        self.start(Step::Connect, "checking the server");
        let (server, entry, previous) = match connect(self.deployable, target, pool, report).await {
            Ok(found) => found,
            Err(err) => return self.stop(Step::Connect, err),
        };
        self.previous = previous.clone();
        self.finish(
            Step::Connect,
            format!("live: {}", previous.as_deref().unwrap_or("nothing")),
        );

        // A backup holds only what was picked: it gets its shared folders
        // and setup again.
        let from_backup = match entry.kind {
            Kind::Compressed => {
                let file = format!("backups/{target}.tar.zst");
                self.start(Step::Unpack, &format!("unpacking {file}"));
                let shared = &self.config().service.shared;
                let unpacked = async {
                    server.restore(target, report).await?;
                    server.link_shared(target, shared, report).await
                }
                .await;
                if let Err(err) = unpacked {
                    let _ = server.drop_open(target, report).await;
                    return self.stop(Step::Unpack, err);
                }
                self.finish(Step::Unpack, format!("from {file}"));
                true
            }
            _ => {
                self.skip(Step::Unpack, "already unpacked");
                false
            }
        };

        if !from_backup {
            self.skip(Step::Setup, "already set up");
        } else if !self.has_setup() {
            self.skip(Step::Setup, "nothing to run");
        } else {
            self.start(Step::Setup, &format!("setting up releases/{target}"));
            match self.set_up(&server, target).await {
                Ok(summary) => self.finish(Step::Setup, summary),
                Err(err) => {
                    let _ = server.drop_open(target, report).await;
                    report.log(
                        LogKind::Info,
                        format!("releases/{target} was removed; its backup stays"),
                    );
                    return self.stop(Step::Setup, err);
                }
            }
        }

        self.start(Step::Switch, "making it the current release");
        if let Err(err) = restore(&server, target, report).await {
            return self.recover(&server, Step::Switch, err).await;
        }
        self.finish(Step::Switch, format!("current → {target}"));

        self.start(Step::Restart, &self.config().service.name);
        if let Err(err) = server.restart(report).await {
            return self.recover(&server, Step::Restart, err).await;
        }
        self.finish(Step::Restart, "");

        self.start(Step::Health, "waiting for the release to settle");
        match self.health(&server).await {
            Ok(summary) => self.finish(Step::Health, summary),
            Err(err) => return self.recover(&server, Step::Health, err).await,
        }

        self.done(&server).await;
        Ended::ok(target)
    }
}

/// Connects, checks sudo, and finds the target and the live release.
async fn connect(
    deployable: &Deployable,
    target: &str,
    pool: &Pool,
    report: &Reporter,
) -> Result<(Server, ReleaseEntry, Option<String>)> {
    let config = &deployable.config;
    let endpoint = &deployable.endpoint;
    report.log(LogKind::Command, format!("connect {}", endpoint.label()));
    let session = pool.get(endpoint).await?;
    let server = Server::new(session, &config.service);
    let facts = server.facts(&[]).await?;
    if !facts.sudo_missing.is_empty() {
        for command in &facts.sudo_missing {
            report.log(
                LogKind::Warn,
                format!("{} may not run: sudo {command}", endpoint.user),
            );
        }
        bail!(
            "{} may not run {} of the commands shipdeck needs as root (sudo, without a password)",
            endpoint.user,
            facts.sudo_missing.len()
        );
    }
    let listing = server.list().await?;
    let Some(entry) = listing.entries.iter().find(|e| e.id == target) else {
        bail!("{target} is no longer on the server");
    };
    if entry.current {
        bail!("{target} is already the current release");
    }
    if entry.name.is_none() {
        bail!("shipdeck did not create {target}, so it will not switch to it");
    }
    Ok((server, entry.clone(), listing.current.clone()))
}
