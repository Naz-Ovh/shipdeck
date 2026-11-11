//! Steps 6 to 10: switch, restart, nginx, health and done.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use super::report::LogKind;
use super::run::{Ended, Run};
use super::steps::{self, Step};
use crate::remote::Server;

impl Run<'_> {
    /// Setup: your commands, while the old release still serves.
    pub(super) async fn step_setup(&mut self, _server: &Server) -> Result<(), Ended> {
        self.start(Step::Setup, "setting up the new release");
        let has = steps::scopes(self.deployment, Step::Setup)
            .iter()
            .any(|scope| !scope.local.is_empty() || !scope.server.is_empty());
        let ran = Box::pin(async {
            if has {
                self.exec(Step::Setup).step(Step::Setup, None).await?;
                self.check_lock()?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        match ran {
            Ok(()) if has => {
                self.finish(Step::Setup, "");
                Ok(())
            }
            Ok(()) => {
                self.skip(Step::Setup, "nothing to run");
                Ok(())
            }
            Err(err) => Err(self.stop(Step::Setup, &err)),
        }
    }

    /// Switch: point `current` at the new release and install its units.
    pub(super) async fn step_switch(&mut self, server: &Server) -> Result<(), Ended> {
        self.start(Step::Switch, "making it the current release");
        let Some(id) = self.release.clone() else {
            return Err(self.stop(Step::Switch, &anyhow!("the release is not named")));
        };
        let switched = Box::pin(async {
            self.exec(Step::Switch).step(Step::Switch, None).await?;
            self.check_lock()?;
            server.switch(&id, self.report).await?;
            for name in self.deployment.process_apps().map(|(name, _)| name) {
                self.exec(Step::Switch)
                    .server_for(Some(name))?
                    .install_unit(self.report)
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        match switched {
            Ok(()) => {
                self.finish(Step::Switch, format!("current -> {id}"));
                Ok(())
            }
            Err(err) => Err(self.recover(Step::Switch, err).await),
        }
    }

    /// Restart: bring each process up on the new release.
    pub(super) async fn step_restart(&mut self) -> Result<(), Ended> {
        self.start(Step::Restart, "restarting the processes");
        let restarted = Box::pin(async {
            self.exec(Step::Restart).step(Step::Restart, None).await?;
            self.check_lock()?;
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
            Ok::<_, anyhow::Error>(names.len())
        })
        .await;
        match restarted {
            Ok(0) => {
                self.finish(Step::Restart, "static apps: nothing to restart");
                Ok(())
            }
            Ok(count) => {
                self.finish(Step::Restart, crate::config::count(count, "process"));
                Ok(())
            }
            Err(err) => Err(self.recover(Step::Restart, err).await),
        }
    }

    /// Nginx: nothing to do, because a deploy with `nginx:` stops at preflight.
    pub(super) async fn step_nginx(&mut self, _server: &Server) -> Result<(), Ended> {
        self.start(Step::Nginx, "checking nginx");
        let ran = self.exec(Step::Nginx).step(Step::Nginx, None).await;
        match ran {
            Ok(()) => {
                self.skip(Step::Nginx, "no app in this deploy asks for nginx");
                Ok(())
            }
            Err(err) => Err(self.recover(Step::Nginx, err).await),
        }
    }

    /// Health: each try runs the commands, then fetches the app's URL.
    pub(super) async fn step_health(&mut self, server: &Server) -> Result<(), Ended> {
        self.start(Step::Health, "waiting for the release to settle");
        let checked = Box::pin(self.health(server)).await;
        match checked {
            Ok(detail) => {
                self.finish(Step::Health, detail);
                Ok(())
            }
            Err(err) => Err(self.recover(Step::Health, err).await),
        }
    }

    async fn health(&self, _server: &Server) -> Result<String> {
        let timeout = Duration::from_secs(self.deployment.health_timeout());
        let deadline = tokio::time::Instant::now() + timeout;
        let mut checked = 0usize;
        let names: Vec<String> = self.deployment.deploy.apps.keys().cloned().collect();
        for name in &names {
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
                        self.release.as_deref(),
                        self.version.as_deref(),
                        Some(name),
                    )
                })
                .transpose()?;
            loop {
                let exec = self.exec(Step::Health);
                let try_once = Box::pin(async {
                    exec.server(Step::Health, scope, None).await?;
                    exec.local(Step::Health, scope, None).await?;
                    if let Some(url) = &url {
                        exec.server_for(Some(name))?.fetch(url).await?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
                match try_once {
                    Ok(()) => {
                        checked += 1;
                        break;
                    }
                    Err(err) => {
                        let message = format!("{err:#}");
                        if tokio::time::Instant::now() + Duration::from_secs(1) >= deadline {
                            bail!(
                                "app {name} was not healthy within {}s: {message}",
                                timeout.as_secs()
                            );
                        }
                        self.report.log(
                            LogKind::Warn,
                            format!("app {name}: {message}; trying again"),
                        );
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
        if !release.server.is_empty() || !release.local.is_empty() {
            loop {
                let exec = self.exec(Step::Health);
                let try_once = Box::pin(async {
                    exec.server(Step::Health, release, None).await?;
                    exec.local(Step::Health, release, None).await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await;
                match try_once {
                    Ok(()) => break,
                    Err(err) => {
                        if tokio::time::Instant::now() + Duration::from_secs(1) >= deadline {
                            bail!(
                                "the release health commands failed within {}s: {err:#}",
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
        let summary = if checked == 0 {
            "nothing to check".to_string()
        } else {
            format!("{} checked", crate::config::count(checked, "app"))
        };
        Ok(summary)
    }

    /// Done: tidy old releases, then your commands.
    pub(super) async fn step_done(&mut self, server: &Server) -> Result<(), Ended> {
        self.start(Step::Done, "tidying old releases");
        let done = Box::pin(async {
            self.exec(Step::Done).step(Step::Done, None).await?;
            self.check_lock()?;
            if self.action == super::steps::Action::Deploy {
                match crate::remote::plan_rotation(&server.list().await?, self.deployment.keep()) {
                    plan if plan.is_empty() => {}
                    plan => {
                        for id in &plan.close {
                            server.drop_open(id, self.report).await?;
                        }
                        for id in &plan.drop_open {
                            server.drop_open(id, self.report).await?;
                        }
                        for id in &plan.drop_backup {
                            server.drop_backup(id, self.report).await?;
                        }
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        match done {
            Ok(()) => {
                self.finish(Step::Done, "");
                Ok(())
            }
            Err(err) => Err(self.stop(Step::Done, &err)),
        }
    }
}
