//! How a run logs in, and which server it reaches: the login flags, the
//! `SHIPDECK_*` variables, and the `--host`, `--port` and `--user` flags.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;

use shipdeck_engine::config::Env;
use shipdeck_engine::ssh::Login;
use shipdeck_protocol::message::SshOverride;
use shipdeck_protocol::message::common::Login as ProtocolLogin;

/// The password, for runs where `--password` would land in a log.
pub const PASSWORD_ENV: &str = "SHIPDECK_PASSWORD";
/// A private key's text, such as a CI secret, kept out of `ps` and logs.
pub const KEY_ENV: &str = "SHIPDECK_SSH_KEY";

#[derive(Args, Clone)]
pub struct LoginArgs {
    /// Log in with this private key file
    #[arg(long, value_name = "PATH", conflicts_with_all = ["password", "key_text"])]
    pub key_file: Option<PathBuf>,

    /// Log in with this private key's text (it shows in `ps`; `SHIPDECK_SSH_KEY` does not)
    #[arg(long, value_name = "TEXT", conflicts_with = "password")]
    pub key_text: Option<String>,

    /// Log in with this password (it lands in your shell history; `SHIPDECK_PASSWORD` does not)
    #[arg(long, value_name = "PASSWORD")]
    pub password: Option<String>,

    /// Log in with no password or key, for a server that allows it
    #[arg(long, conflicts_with_all = ["key_file", "key_text", "password"])]
    pub passwordless: bool,

    /// Reach this server instead of `server.ssh.host`; for one deploy (or `SHIPDECK_SSH_HOST`)
    #[arg(short = 'H', long, value_name = "HOST")]
    pub host: Option<String>,

    /// Use this SSH port instead of `server.ssh.port`; for one deploy (or `SHIPDECK_SSH_PORT`)
    #[arg(short = 'P', long, value_name = "PORT")]
    pub port: Option<String>,

    /// Log in as this user instead of `server.ssh.user`; for one deploy (or `SHIPDECK_SSH_USER`)
    #[arg(short = 'U', long, value_name = "USER")]
    pub user: Option<String>,

    /// Run server commands without the login shell's startup files (~/.bashrc, ~/.zshrc, ...), on every server
    #[arg(long)]
    pub no_shell_env: bool,

    /// Run local commands with this sh.exe instead of the one Git for Windows has
    #[cfg(windows)]
    #[arg(long, value_name = "PATH", env = "SHIPDECK_GITSHELL_PATH")]
    pub gitshell_path: Option<PathBuf>,
}

impl LoginArgs {
    /// The server flags, as the engine takes them.
    pub fn ssh(&self) -> SshOverride {
        SshOverride {
            host: self.host.clone(),
            port: self.port.clone(),
            user: self.user.clone(),
        }
    }
}

/// The shell the engine runs local commands in, when the flag names one.
#[cfg(windows)]
pub fn gitshell(args: &LoginArgs) -> Option<PathBuf> {
    args.gitshell_path.clone()
}

/// Windows has the flag only; the other systems use their own `sh`.
#[cfg(not(windows))]
pub fn gitshell(_: &LoginArgs) -> Option<PathBuf> {
    None
}

/// The login as the protocol carries it: the same four ways in.
pub fn wire(login: &Login) -> ProtocolLogin {
    match login {
        Login::Key(path) => ProtocolLogin::Key {
            path: path.display().to_string(),
        },
        Login::KeyText(text) => ProtocolLogin::KeyText { text: text.clone() },
        Login::Password(password) => ProtocolLogin::Password {
            password: password.clone(),
        },
        Login::Passwordless => ProtocolLogin::Passwordless,
        Login::Auto => ProtocolLogin::Auto,
    }
}

/// The first login that is given: a flag, then a variable of `env`.
///
/// # Errors
/// Returns an error when `--key-file` names a file that is not there.
pub fn login(args: LoginArgs, env: &Env) -> Result<Login> {
    if args.passwordless {
        return Ok(Login::Passwordless);
    }
    if let Some(key) = args.key_file {
        let key = expand_home(key);
        if !key.is_file() {
            bail!("the key file {} does not exist", key.display());
        }
        return Ok(Login::Key(key));
    }
    let set = |name: &str| env.get(name).filter(|value| !value.trim().is_empty());
    if let Some(text) = args.key_text {
        return Ok(Login::KeyText(text));
    }
    if let Some(password) = args.password {
        return Ok(Login::Password(password));
    }
    if let Some(text) = set(KEY_ENV) {
        return Ok(Login::KeyText(text.to_string()));
    }
    Ok(match set(PASSWORD_ENV) {
        Some(password) => Login::Password(password.to_string()),
        None => Login::Auto,
    })
}

/// `~/.ssh/id_ed25519` → `/home/me/.ssh/id_ed25519`; zsh expands `~` only
/// at the start of a word, not after `--key-file=`.
fn expand_home(path: PathBuf) -> PathBuf {
    match (path.strip_prefix("~"), std::env::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path,
    }
}

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
