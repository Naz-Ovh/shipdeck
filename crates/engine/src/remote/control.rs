//! The service by hand, outside a deploy: stop, start and restart it, and
//! follow its log live. With systemd, stopping, starting and the log need
//! sudo rights of their own ([`control_commands`]); a server set up before
//! them still deploys, and only these say `server-setup.sh` must run again.

use std::future::Future;

use anyhow::{Result, bail};

use super::Server;
use crate::config::Manager;
use crate::setup;
use crate::ssh::sh_quote;

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
pub fn control_commands(service: &str) -> Vec<String> {
    vec![
        format!("systemctl stop {service}"),
        format!("systemctl start {service}"),
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
fn log_script(manager: Manager, service: &str, pm2_app: &str, sudo: &str) -> String {
    match manager {
        Manager::Systemd => format!("{sudo}{}", journal_follow(&sh_quote(service))),
        Manager::Pm2 => format!("pm2 logs {} --raw --lines {LOG_LINES}", sh_quote(pm2_app)),
    }
}

impl Server {
    /// What follows the service's log, as the log view names it.
    pub fn log_command(&self) -> String {
        match self.manager {
            Manager::Systemd => journal_follow(&self.service),
            Manager::Pm2 => format!("pm2 logs {} --raw --lines {LOG_LINES}", self.pm2_app),
        }
    }

    /// Stops, starts or restarts the service. A PM2 app that is already
    /// running is left as it is by `Start`.
    pub async fn control(&self, action: Control) -> Result<()> {
        let script = match self.manager {
            Manager::Systemd => {
                self.check_sudo(|service| systemctl(action, service))
                    .await?;
                let command = systemctl(action, &sh_quote(&self.service));
                format!("{}{command}", self.sudo())
            }
            Manager::Pm2 => match action {
                Control::Stop => format!("pm2 stop {}", sh_quote(&self.pm2_app)),
                Control::Start if self.pm2_state().await?.0 == "active" => return Ok(()),
                Control::Start => self.pm2_script("start"),
                Control::Restart => self.pm2_script("startOrReload"),
            },
        };
        self.session
            .exec(&script)
            .await?
            .check(&format!("{} {}", action.verb(), self.service))?;
        Ok(())
    }

    /// Follows the service's log, handing each line to `on_line`, until
    /// `stop` resolves (which ends the remote command) or the log ends on
    /// its own; then returns its exit code.
    pub async fn follow_logs(
        &self,
        on_line: impl FnMut(&str),
        stop: impl Future<Output = ()>,
    ) -> Result<Option<u32>> {
        if self.manager == Manager::Systemd {
            self.check_sudo(journal_follow).await?;
        }
        let script = log_script(self.manager, &self.service, &self.pm2_app, self.sudo());
        self.session.follow(&script, on_line, stop).await
    }

    /// Fails, saying what to run, when the user may not run the service's
    /// `command` with sudo. Root needs nothing.
    async fn check_sudo(&self, command: impl Fn(&str) -> String) -> Result<()> {
        if self.is_root() {
            return Ok(());
        }
        let check = format!(
            "sudo -n -l {} >/dev/null 2>&1",
            command(&sh_quote(&self.service))
        );
        if self.session.exec(&check).await?.success() {
            return Ok(());
        }
        bail!(
            "{user} may not run sudo {command}. Run server-setup.sh on the server again, \
             as root, to allow it:\n{setup}",
            user = self.session.user,
            command = command(&self.service),
            setup = setup::command(&self.session.user, &self.dir, &self.service),
        )
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod tests;
