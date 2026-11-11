//! Where and as whom to log in: `shipdeck.yaml`'s `ssh` block with
//! `~/.ssh/config` applied, so `"host": "prod"` works like `ssh prod`.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use super::ssh_config::{self, HostConfig};
use crate::config::{Port, SshTarget};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// `ssh.host` as written in shipdeck.yaml: an alias or a real address.
    pub name: String,
    /// The address to connect to: `HostName` for an alias, else `name`.
    pub host: String,
    pub port: u16,
    pub user: String,
    /// `IdentityFile` from `~/.ssh/config`, used when no `--key-file` or
    /// `--password` is given.
    pub identity: Option<PathBuf>,
    /// `User` from `~/.ssh/config`, which `shipdeck.yaml` may have overridden.
    pub config_user: Option<String>,
    /// `ssh.shell_env`: commands get the login shell's environment.
    pub shell_env: bool,
}

impl Endpoint {
    /// Applies `~/.ssh/config` to `target`. Values in shipdeck.yaml win.
    ///
    /// # Errors
    ///
    /// Fails when `~/.ssh/config` routes the host through a proxy, or no
    /// user name can be found or allowed.
    pub fn resolve(target: &SshTarget) -> Result<Self> {
        Self::from_config(target, ssh_config::lookup(&target.host))
    }

    /// Builds the endpoint from the deploy target and its `~/.ssh/config`
    /// entry; values in shipdeck.yaml win.
    ///
    /// # Errors
    ///
    /// Fails when the host is reached through a proxy, or the user is
    /// missing or holds characters an ssh command cannot carry.
    pub fn from_config(target: &SshTarget, found: HostConfig) -> Result<Self> {
        if let Some(proxy) = &found.proxy {
            bail!(
                "~/.ssh/config reaches {} through `{proxy}`, which shipdeck cannot follow yet; \
                 put the server's own address in ssh.host",
                target.host
            );
        }
        let user = target.user.clone().or(found.user.clone()).ok_or_else(|| {
            anyhow!(
                "`ssh.user` is not set, and ~/.ssh/config has no User for {}",
                target.host
            )
        })?;
        if user.is_empty()
            || !user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            bail!("the SSH user {user:?} may only use letters, digits, `.`, `_` and `-`");
        }
        let port = match &target.port {
            Some(Port::Number(number)) => Some(*number),
            Some(Port::Text(text)) => bail!("`ssh.port` is {text:?}, and it is not a port number"),
            None => found.port,
        };
        Ok(Self {
            host: found.host_name.unwrap_or_else(|| target.host.clone()),
            port: port.unwrap_or(22),
            name: target.host.clone(),
            user,
            identity: found.identity_file,
            config_user: found.user,
            shell_env: target.shell_env,
        })
    }

    /// A stand-in for a deploy that is unavailable: what its file says, never
    /// dialled.
    #[must_use]
    pub fn placeholder(target: &SshTarget) -> Self {
        Self {
            name: target.host.clone(),
            host: target.host.clone(),
            port: 22,
            user: target.user.clone().unwrap_or_default(),
            identity: None,
            config_user: None,
            shell_env: target.shell_env,
        }
    }

    /// `deploy@prod`, or `deploy@prod:2222` on another port.
    #[must_use]
    pub fn label(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.user, self.name)
        } else {
            format!("{}@{}:{}", self.user, self.name, self.port)
        }
    }

    /// Whether `name` is an alias for a different address.
    #[must_use]
    pub fn is_alias(&self) -> bool {
        self.host != self.name
    }

    /// `203.0.113.10:22`, the address actually dialled.
    #[must_use]
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
#[path = "endpoint_test.rs"]
mod tests;
