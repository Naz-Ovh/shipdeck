//! A small lease file that marks a server command still running for a
//! project. A new operation refuses to start while the command lives, even
//! when the agent connection that held the project lock is already gone.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use super::Server;
use crate::ssh::sh_quote;

/// How long the lease check may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

impl Server {
    /// Whether a server command for this project is still running.
    ///
    /// # Errors
    /// Returns an error when the lease file cannot be checked.
    pub async fn command_lease_active(&self) -> Result<bool> {
        let lease = sh_quote(&format!("{}/.shipdeck.command", self.dir));
        let script = format!(
            "lease={lease}
if [ -f \"$lease\" ]; then
  read -r pid token < \"$lease\" || true
  if [ -n \"$pid\" ] && {{ kill -0 \"$pid\" 2>/dev/null || kill -0 \"-$pid\" 2>/dev/null; }}; then
    if [ -n \"$token\" ] && tr '\\0' '\\n' < \"/proc/$pid/environ\" 2>/dev/null | grep -qx \"SHIPDECK_COMMAND_LEASE=$token\"; then
      echo active
      exit 0
    fi
    echo active
    exit 0
  fi
  rm -f \"$lease\" || true
fi
echo idle"
        );
        let out = self
            .session()
            .exec_raw_within(&script, CHECK_TIMEOUT)
            .await
            .context("checking for a running server command")?;
        Ok(out.stdout.trim() == "active")
    }
}

/// Marks the command's shell for the life of `Server::command`.
pub(super) fn prelude(dir: &str, token: &str) -> String {
    format!(
        "lease={lease}
if [ -f \"$lease\" ]; then
  read -r old_pid old_token < \"$lease\" || true
  if [ -n \"$old_pid\" ] && {{ kill -0 \"$old_pid\" 2>/dev/null || kill -0 \"-$old_pid\" 2>/dev/null; }}; then
    echo 'another server command is still running for this project' >&2
    exit 1
  fi
  rm -f \"$lease\" || true
fi
export SHIPDECK_COMMAND_LEASE={token}
printf '%s %s\\n' \"$$\" \"$SHIPDECK_COMMAND_LEASE\" > \"$lease\"
trap 'rm -f \"$lease\"' EXIT
",
        lease = sh_quote(&format!("{dir}/.shipdeck.command")),
        token = sh_quote(token),
    )
}

/// A unique token for one command lease.
pub(super) fn new_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{}-{nanos}", std::process::id())
}
