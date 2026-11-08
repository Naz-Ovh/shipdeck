//! The explicit user behind app and PM2 commands. The agent may run as root,
//! but these commands never inherit that identity.

use std::process::Command;

use anyhow::{Context, Result, bail};

/// A plain Linux account name.
pub(crate) fn safe_user(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._-".contains(ch))
}

/// One account from the server's user database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

/// The account plus the login environment app commands need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Login {
    pub account: Account,
    pub path: String,
    pub pm2_home: String,
    pub pm2: Option<String>,
}

/// Looks up one account; the name must already be safe.
///
/// # Errors
///
/// Fails when the account does not exist or `getent` cannot read it.
pub(crate) fn lookup(name: &str) -> Result<Account> {
    if !safe_user(name) {
        bail!("invalid user name: {name:?}");
    }
    let output = Command::new("getent")
        .args(["passwd", name])
        .output()
        .context("running getent to find the app user")?;
    if !output.status.success() {
        bail!("the app user {name} does not exist on the server");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .context("the user database returned no account")?;
    let mut fields = line.split(':');
    let _name = fields.next();
    let _password = fields.next();
    let uid = fields
        .next()
        .context("the account has no uid")?
        .parse()
        .context("the account uid is not a number")?;
    let gid = fields
        .next()
        .context("the account has no gid")?
        .parse()
        .context("the account gid is not a number")?;
    let _gecos = fields.next();
    let home = fields
        .next()
        .context("the account has no home folder")?
        .to_string();
    if !home.starts_with('/') {
        bail!("the home folder of {name} is not absolute");
    }
    Ok(Account {
        name: name.to_string(),
        uid,
        gid,
        home,
    })
}

/// Resolves the account and its login `PATH` and `PM2_HOME`.
///
/// # Errors
///
/// Fails when the account is missing or its login shell cannot report where
/// PM2 and Node.js are.
pub(crate) fn resolve(name: &str) -> Result<Login> {
    let account = lookup(name)?;
    if !is_root() && current_user()? != account.name {
        bail!("this agent cannot run app commands as {}", account.name);
    }
    let (path, pm2_home, pm2) = login_environment(name, &account.home)?;
    Ok(Login {
        account,
        path,
        pm2_home,
        pm2,
    })
}

/// Asks the user's login shell for the two values that affect PM2.
fn login_environment(name: &str, home: &str) -> Result<(String, String, Option<String>)> {
    let script = r#"printf 'PATH=%s\n' "$PATH"
printf 'PM2_HOME=%s\n' "${PM2_HOME:-$HOME/.pm2}"
printf 'PM2=%s\n' "$(command -v pm2 2>/dev/null || true)""#;
    let output = Command::new("runuser")
        .args(["-u", name, "--", "sh", "-lc", script])
        .output()
        .context("running the app user's login shell")?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        bail!("the login shell of {name} failed: {}", why.trim());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .map(str::to_string)
    };
    let path = field("PATH=")
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".to_string());
    let pm2_home = field("PM2_HOME=")
        .filter(|home| home.starts_with('/'))
        .unwrap_or_else(|| format!("{home}/.pm2"));
    let pm2 = field("PM2=").filter(|path| path.starts_with('/'));
    Ok((path, pm2_home, pm2))
}

/// Prepares a standard command that runs `program` as this user.
pub(crate) fn command(
    login: &Login,
    program: &str,
    args: &[&str],
    extra: &std::collections::BTreeMap<String, String>,
) -> Command {
    let mut command = if is_root() {
        let mut command = Command::new("runuser");
        command.args(["-u", login.account.name.as_str(), "--", "env", "-i"]);
        command.arg(format!("HOME={}", login.account.home));
        command.arg(format!("PATH={}", login.path));
        command.arg(format!("PM2_HOME={}", login.pm2_home));
        for (key, value) in extra {
            if key != "HOME" && key != "PM2_HOME" {
                command.arg(format!("{key}={value}"));
            }
        }
        command.arg(program);
        command.args(args);
        command
    } else {
        let mut command = Command::new(program);
        command.env_clear();
        command.env("HOME", &login.account.home);
        command.env("PATH", &login.path);
        command.env("PM2_HOME", &login.pm2_home);
        command.envs(extra);
        command.args(args);
        command
    };
    command.stdin(std::process::Stdio::null());
    command
}

/// Whether this process is root. When it is not, the identity must be its own.
pub(crate) fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|text| text.trim() == "0")
}

/// The current user's name, for the non-root fallback.
pub(crate) fn current_user() -> Result<String> {
    let output = Command::new("id")
        .arg("-un")
        .output()
        .context("running id -un")?;
    if !output.status.success() {
        bail!("could not find the current user");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
