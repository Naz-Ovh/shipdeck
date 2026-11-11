//! The service by hand, outside a deploy: stop, start and restart it, and
//! follow its log live. With systemd, stopping, starting and the log need
//! sudo rights of their own ([`control_commands`]); a server set up before
//! them still deploys, and only these say `server-setup.sh` must run again.

use std::future::Future;

use anyhow::{Result, bail};

use super::Server;
use crate::agent::AgentLogEvent;
use crate::config::{Manager, ManagerKind};
use crate::setup;
use crate::ssh::sh_quote;
use shipdeck_protocol::message::agent::AgentServiceAction;

/// Journal or PM2 lines shown before the log follows.
const LOG_LINES: u32 = 200;

/// What `:stop`, `:start` and `:restart` do to the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Stop,
    Start,
    Restart,
}

impl Control {
    #[must_use]
    pub fn verb(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Start => "start",
            Self::Restart => "restart",
        }
    }
}

/// The commands that need root to run the service by hand, without
/// `sudo`. They are kept apart from `sudo_commands`, which preflight
/// requires, so a server set up before them still deploys. The sudoers
/// rule must allow exactly these; changing one breaks every server set up
/// before.
#[must_use]
pub fn control_commands(service: &str) -> Vec<String> {
    vec![
        format!("systemctl stop {service}"),
        format!("systemctl start {service}"),
        format!("systemctl disable {service}"),
        journal_follow(service),
    ]
}

/// The service's last journal lines, then each new one as it comes. With
/// the plain name it is what the sudoers rule names; with the name quoted,
/// what `sh` runs.
fn journal_follow(service: &str) -> String {
    format!("journalctl -u {service} -f -n {LOG_LINES} --no-pager")
}

/// The systemd command for `action`, named like [`journal_follow`].
fn systemctl(action: Control, service: &str) -> String {
    format!("systemctl {} {service}", action.verb())
}

/// What follows the log: the journal through `sudo` (empty for root), or
/// PM2's own logs.
fn log_script(manager: Option<&Manager>, service: &str, pm2_app: &str, sudo: &str) -> String {
    match manager {
        Some(Manager::Systemd(_)) => format!("{sudo}{}", journal_follow(&sh_quote(service))),
        Some(Manager::Pm2(_)) => {
            format!("pm2 logs {} --raw --lines {LOG_LINES}", sh_quote(pm2_app))
        }
        Some(Manager::Oxmgr(_)) | None => String::new(),
    }
}

/// What follows `service`'s log, as a screen names it.
#[must_use]
pub fn log_command(manager: Option<&Manager>, process: &str) -> String {
    match manager {
        Some(Manager::Systemd(_)) => journal_follow(process),
        Some(Manager::Pm2(_)) => format!("pm2 logs {process} --raw --lines {LOG_LINES}"),
        Some(Manager::Oxmgr(_)) | None => "a static site has no process logs".to_string(),
    }
}

impl Server {
    /// Checks stop rights before a deployment changes any live process.
    ///
    /// # Errors
    /// Returns an error when stop or disable is not permitted.
    pub async fn check_stop_permission(&self) -> Result<()> {
        if self.manager.is_none() {
            return Ok(());
        }
        if self.manager_kind() == Some(ManagerKind::Systemd) {
            self.check_sudo(|service| systemctl(Control::Stop, service))
                .await?;
            self.check_sudo(|service| format!("systemctl disable {service}"))
                .await?;
        }
        Ok(())
    }

    /// Removes a process that is absent or uses another manager in the target.
    ///
    /// # Errors
    /// Returns an error when retiring the process fails.
    pub async fn retire(&self, report: &crate::deploy::report::Reporter) -> Result<()> {
        if self.manager.is_none() {
            return Ok(());
        }
        self.check_stop_permission().await?;
        let script = match self.manager {
            Some(Manager::Oxmgr(_)) => bail!("oxmgr is not built yet"),
            None => return Ok(()),
            Some(Manager::Systemd(_)) => format!(
                "set -e\nstate=$(systemctl show {} -p LoadState --value)\nif [ \"$state\" != not-found ]; then {}systemctl stop {}; {}systemctl disable {}; fi",
                self.q_service(),
                self.sudo(),
                self.q_service(),
                self.sudo(),
                self.q_service()
            ),
            Some(Manager::Pm2(_)) => format!(
                "set -e\npids=$(pm2 pid {})\nif [ -n \"$pids\" ]; then pm2 delete {}; fi\npm2 save --force",
                sh_quote(&self.process),
                sh_quote(&self.process)
            ),
        };
        self.run("retire previous process", &script, report).await?;
        Ok(())
    }

    /// Stops, starts or restarts the service. A PM2 app that is already
    /// running is left as it is by `Start`.
    ///
    /// # Errors
    /// Returns an error when sudo is not allowed, the PM2 state is unreadable,
    /// or the command fails.
    pub async fn control(&self, action: Control) -> Result<()> {
        if self.is_static() {
            bail!("a static site has no process to {}", action.verb());
        }
        if let Some(agent) = self.session.agent() {
            let action = match action {
                Control::Stop => AgentServiceAction::Stop,
                Control::Start => AgentServiceAction::Start,
                Control::Restart => AgentServiceAction::Restart,
            };
            agent
                .service_control(&self.agent_service()?, action)
                .await?;
            return Ok(());
        }
        let script = match self.manager {
            None => return Ok(()),
            Some(Manager::Oxmgr(_)) => bail!("oxmgr is not built yet"),
            Some(Manager::Systemd(_)) => {
                self.check_sudo(|service| systemctl(action, service))
                    .await?;
                let command = systemctl(action, &self.q_service());
                format!("{}{command}", self.sudo())
            }
            Some(Manager::Pm2(_)) => match action {
                Control::Stop => format!("pm2 stop {}", sh_quote(&self.process)),
                Control::Start if self.pm2_state().await?.0 == "active" => return Ok(()),
                Control::Start => self.pm2_script("start"),
                Control::Restart => self.pm2_script("startOrReload"),
            },
        };
        self.session
            .exec(&script)
            .await?
            .check(&format!("{} {}", action.verb(), self.process))?;
        Ok(())
    }

    /// Follows the service's log, handing each line to `on_line`, until
    /// `stop` resolves (which ends the remote command) or the log ends on
    /// its own; then returns its exit code.
    ///
    /// # Errors
    /// Returns an error when sudo is not allowed or the log command cannot run.
    pub async fn follow_logs(
        &self,
        on_line: impl FnMut(&str),
        stop: impl Future<Output = ()>,
    ) -> Result<Option<u32>> {
        if self.is_static() {
            bail!("a static site has no process logs; nginx keeps its own in /var/log/nginx");
        }
        if let Some(agent) = self.session.agent() {
            let (view, mut events) = agent.open_log(&self.agent_service()?, LOG_LINES).await?;
            let mut on_line = on_line;
            let mut stop = std::pin::pin!(stop);
            loop {
                tokio::select! {
                    () = &mut stop => {
                        agent.stop_log(view).await;
                        return Ok(None);
                    }
                    event = events.recv() => match event {
                        Some(AgentLogEvent::Line(line)) => on_line(&line),
                        Some(AgentLogEvent::Ended { code, error }) => {
                            if let Some(error) = error {
                                bail!("{error}");
                            }
                            return Ok(code);
                        }
                        None => bail!("the agent connection ended while following the log"),
                    },
                }
            }
        }
        if self.manager_kind() == Some(ManagerKind::Systemd) {
            self.check_sudo(journal_follow).await?;
        }
        let script = log_script(
            self.manager.as_ref(),
            &self.process,
            &self.process,
            self.sudo(),
        );
        self.session.follow(&script, on_line, stop).await
    }

    /// Fails, saying what to run, when the user may not run the service's
    /// `command` with sudo. Root needs nothing.
    async fn check_sudo(&self, command: impl Fn(&str) -> String) -> Result<()> {
        if self.is_root() {
            return Ok(());
        }
        let check = format!("sudo -n -l {} >/dev/null 2>&1", command(&self.q_service()));
        if self.session.exec(&check).await?.success() {
            return Ok(());
        }
        bail!(
            "{user} may not run sudo {command}. Run server-setup.sh on the server again, \
             as root, to allow it:\n{setup}",
            user = self.session.user,
            command = command(&self.process),
            setup = setup::command(&self.session.user, &self.dir, &self.process),
        )
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod tests;
