//! Where agents live on a server, whether the connection may use root, and
//! how an installed agent is found and started.

use std::fmt::Write as _;

use anyhow::{Context, Result, bail};

use super::version::{Installed, Version};
use crate::ssh::{Session, sh_quote};

/// The root-owned folder holding every versioned agent installation.
pub(super) const AGENT_ROOT: &str = "/opt/shipdeck/agent";

/// The root-owned directory for one release label.
pub(super) fn version_dir(label: &str) -> String {
    format!("{AGENT_ROOT}/v{label}")
}

/// The full executable path for one release label.
pub(super) fn binary_path(label: &str) -> String {
    format!("{}/agent", version_dir(label))
}

/// `sudo -n -- ` when the account is not root, or nothing for root.
///
/// # Errors
///
/// Fails with a clear message when the account cannot run privileged commands
/// without a password. The agent installation and systemd work need them.
pub(super) async fn sudo_prefix(session: &Session) -> Result<String> {
    if session.user == "root" {
        return Ok(String::new());
    }
    let output = session
        .exec_raw("sudo -n -- sh -c 'id -u' 2>/dev/null")
        .await
        .context("checking sudo access for the agent")?;
    if output.success() && output.stdout.trim() == "0" {
        return Ok("sudo -n -- ".to_string());
    }
    let detail = output.stderr.trim();
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" ({detail})")
    };
    bail!(
        "user {} cannot run the privileged commands the shipdeck agent needs{detail}. \
         The agent installs under {AGENT_ROOT} and manages systemd, so the account must be \
         root or have password-free sudo (NOPASSWD).",
        session.user
    );
}

/// The SSH user's home, as an absolute path.
pub(super) async fn home(session: &Session) -> Result<String> {
    let home = session
        .exec_raw("printf '%s' \"$HOME\"")
        .await
        .context("asking the server for the home folder")?
        .stdout;
    let home = home.trim();
    if !home.starts_with('/') {
        bail!("the server did not say where the home folder is");
    }
    Ok(home.to_string())
}

/// Every valid root-owned agent installation, oldest first.
///
/// A directory that is missing, unsafe, or whose binary reports another
/// version is not counted as installed. Its binary is never executed unless
/// its version directory, the installation root, and the executable are all
/// root-owned and not writable by other users.
pub(super) async fn installed(session: &Session, prefix: &str) -> Result<Vec<Installed>> {
    let output = session
        .exec_raw(&privileged(prefix, LIST_SCRIPT))
        .await
        .context("listing installed agents")?
        .check("listing installed agents")?;
    let mut found = Vec::new();
    for name in output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let Some(text) = name.strip_prefix('v') else {
            continue;
        };
        let Ok(version) = Version::parse(text) else {
            continue;
        };
        let path = format!("{AGENT_ROOT}/{name}/agent");
        if probe(session, prefix, &path, text).await? {
            found.push(Installed {
                version,
                label: text.to_string(),
                path,
            });
        }
    }
    found.sort_by_key(|candidate| candidate.version);
    Ok(found)
}

/// Runs the probe for one candidate; a failed probe means "not installed".
async fn probe(session: &Session, prefix: &str, path: &str, label: &str) -> Result<bool> {
    let script = probe_script(path);
    let output = session
        .exec_raw(&privileged(prefix, &script))
        .await
        .with_context(|| format!("checking the agent at {path}"))?;
    Ok(output.success() && output.stdout.trim() == format!("shipdeck-agent {label}"))
}

/// Wraps a fixed script for root access.
pub(super) fn privileged(prefix: &str, script: &str) -> String {
    format!("{prefix}sh -c {}", sh_quote(script))
}

const LIST_SCRIPT: &str = "\
set -eu
[ -d /opt/shipdeck/agent ] || exit 0
for path in /opt/shipdeck/agent/v*; do
  [ -d \"$path\" ] || continue
  [ -L \"$path\" ] && continue
  printf '%s\\n' \"${path##*/}\"
done
";

/// Checks every directory above the executable before running it.
fn probe_script(path: &str) -> String {
    format!(
        r#"set -eu
path={path}
[ -f "$path" ] || exit 3
[ ! -L "$path" ] || exit 4
[ "$(stat -c %U -- "$path")" = root ] || exit 5
mode=$(stat -c %a -- "$path")
[ $((0$mode & 022)) -eq 0 ] || exit 6
[ -x "$path" ] || exit 7
dir=$(dirname -- "$path")
while [ "$dir" != / ]; do
  [ -d "$dir" ] || exit 8
  [ ! -L "$dir" ] || exit 9
  [ "$(stat -c %U -- "$dir")" = root ] || exit 10
  mode=$(stat -c %a -- "$dir")
  [ $((0$mode & 022)) -eq 0 ] || exit 11
  dir=$(dirname -- "$dir")
done
out=$("$path" --version 2>/dev/null) || exit 12
printf '%s' "$out"
"#,
        path = sh_quote(path),
    )
}

/// The command that starts `path` as root and keeps the SSH user's home for
/// agent state and its log.
pub(super) fn launch_command(prefix: &str, path: &str, operator: &str, home: &str) -> String {
    agent_command(prefix, path, operator, home, "")
}

/// The command that starts the upload receiver as root, with the same
/// identity and state home as the control agent.
pub(super) fn receiver_command(
    prefix: &str,
    path: &str,
    operator: &str,
    home: &str,
    reservation: &str,
) -> String {
    let args = format!("--receive {}", sh_quote(reservation));
    agent_command(prefix, path, operator, home, &args)
}

fn agent_command(prefix: &str, path: &str, operator: &str, home: &str, args: &str) -> String {
    let log = format!("{home}/.shipdeck-agent.log");
    let mut inner = format!(
        "exec {prefix}env SHIPDECK_OPERATOR={operator} SHIPDECK_OPERATOR_HOME={home} {path}",
        operator = sh_quote(operator),
        home = sh_quote(home),
        path = sh_quote(path),
    );
    if !args.is_empty() {
        inner.push(' ');
        inner.push_str(args);
    }
    let _ = write!(inner, " 2>>{}", sh_quote(&log));
    format!("sh -c {}", sh_quote(&inner))
}

#[cfg(test)]
#[path = "layout_test.rs"]
mod tests;
