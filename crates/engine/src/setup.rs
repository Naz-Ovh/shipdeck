//! The one-time server setup. `server-setup.sh` lives at the root of the
//! shipdeck repository; a server fetches it at the release tag of this build
//! and runs it once as root with one deployable's values. It creates the
//! deploy user, authorizes its key, creates the app folder and installs the
//! sudo rule shipdeck needs.

use std::path::{Path, PathBuf};

use crate::config::Deployable;
use crate::ssh::{Login, sh_quote};

pub const SCRIPT_NAME: &str = "server-setup.sh";

/// What to run on the server, and what it was built from.
#[derive(Debug, Clone)]
pub struct SetupCommand {
    /// `curl -fsSL <url> | bash -s -- --user ... --dir ... --service ... [--key ...]`
    pub command: String,
    pub url: String,
    /// The private key whose public half the command authorizes, if any.
    pub key: Option<PathBuf>,
}

/// The script at the tag of this shipdeck version, so what runs as root only
/// changes with a new release.
pub fn script_url() -> String {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let path = repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository);
    format!(
        "https://raw.githubusercontent.com/{path}/v{}/{SCRIPT_NAME}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The command for one deployable. The key is the public half of `--key`,
/// else of the alias's `IdentityFile`.
pub fn command(deployable: &Deployable, login: &Login) -> SetupCommand {
    let key = match login {
        Login::Key(path) => Some(path.clone()),
        Login::Password(_) | Login::Auto => deployable.endpoint.identity.clone(),
    };
    let public = key.as_deref().and_then(public_key);
    let args = arguments(deployable, public.as_deref())
        .iter()
        .map(|arg| {
            if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./@".contains(c))
            {
                arg.clone()
            } else {
                sh_quote(arg)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let url = script_url();
    SetupCommand {
        command: format!("curl -fsSL {url} | bash -s -- {args}"),
        url,
        key: key.filter(|_| public.is_some()),
    }
}

/// The script's arguments, unquoted.
pub fn arguments(deployable: &Deployable, public_key: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--user".to_string(),
        deployable.endpoint.user.clone(),
        "--dir".to_string(),
        deployable.config.remote.dir.clone(),
        "--service".to_string(),
        deployable.config.remote.service.clone(),
    ];
    if let Some(key) = public_key {
        args.push("--key".to_string());
        args.push(key.to_string());
    }
    args
}

/// The public half of a private key: its `.pub` file, else read from an
/// OpenSSH private key (which stores it unencrypted).
pub fn public_key(private_key: &Path) -> Option<String> {
    let pub_file = PathBuf::from(format!("{}.pub", private_key.display()));
    if let Ok(text) = std::fs::read_to_string(&pub_file) {
        return text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(str::to_string);
    }
    let key = russh::keys::ssh_key::PrivateKey::read_openssh_file(private_key).ok()?;
    key.public_key().to_openssh().ok()
}

/// sudo skips files in `/etc/sudoers.d` whose names contain a `.`.
pub fn sudoers_file(service: &str) -> String {
    format!("/etc/sudoers.d/shipdeck-{}", service.replace('.', "_"))
}

#[cfg(test)]
#[path = "setup_test.rs"]
mod tests;
