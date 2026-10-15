//! Where and as whom to log in: `deploy.yaml`'s `ssh` block with
//! `~/.ssh/config` applied, so `"host": "prod"` works like `ssh prod`.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use super::ssh_config::{self, HostConfig};
use crate::config::SshTarget;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// `ssh.host` as written in deploy.yaml: an alias or a real address.
    pub name: String,
    /// The address to connect to: `HostName` for an alias, else `name`.
    pub host: String,
    pub port: u16,
    pub user: String,
    /// `IdentityFile` from `~/.ssh/config`, used when no `--key` or
    /// `--password` is given.
    pub identity: Option<PathBuf>,
    /// `User` from `~/.ssh/config`, which `deploy.yaml` may have overridden.
    pub config_user: Option<String>,
    /// `ssh.shell_env`: commands get the login shell's environment.
    pub shell_env: bool,
}

impl Endpoint {
    /// Applies `~/.ssh/config` to `target`. Values in deploy.yaml win.
    pub fn resolve(target: &SshTarget) -> Result<Self> {
        Self::from_config(target, ssh_config::lookup(&target.host))
    }

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
        Ok(Self {
            host: found.host_name.unwrap_or_else(|| target.host.clone()),
            port: target.port.or(found.port).unwrap_or(22),
            name: target.host.clone(),
            user,
            identity: found.identity_file,
            config_user: found.user,
            shell_env: target.shell_env,
        })
    }

    /// `deploy@prod`, or `deploy@prod:2222` on another port.
    pub fn label(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.user, self.name)
        } else {
            format!("{}@{}:{}", self.user, self.name, self.port)
        }
    }

    /// Whether `name` is an alias for a different address.
    pub fn is_alias(&self) -> bool {
        self.host != self.name
    }

    /// `203.0.113.10:22`, the address actually dialled.
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
#[path = "endpoint_test.rs"]
mod tests;
