//! Logging in with a key file, a password from the command line or
//! SHIPDECK_PASSWORD, or a password asked for in the TUI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use russh::client::Handle;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, load_secret_key};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::Endpoint;
use super::handler::Client;

const ATTEMPTS: usize = 3;

/// How to log in, from the command-line flags.
#[derive(Debug, Clone)]
pub enum Login {
    Key(PathBuf),
    Password(String),
    /// The `IdentityFile` from `~/.ssh/config` if the host has one, then a
    /// password asked in the TUI.
    Auto,
}

impl Login {
    pub fn describe(&self) -> String {
        match self {
            Login::Key(path) => format!("key {}", path.display()),
            Login::Password(_) => "password".into(),
            Login::Auto => "~/.ssh/config key, else a password".into(),
        }
    }
}

/// Something the SSH layer needs the person at the keyboard to answer.
#[derive(Debug)]
pub enum Question {
    Secret {
        title: String,
        prompt: String,
        reply: oneshot::Sender<Option<String>>,
    },
    TrustHost {
        host: String,
        algorithm: String,
        fingerprint: String,
        reply: oneshot::Sender<bool>,
    },
}

/// Passwords and passphrases typed during this run, so each is asked once.
#[derive(Default)]
pub struct Secrets(Mutex<HashMap<String, String>>);

impl Secrets {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().expect("secrets lock").get(key).cloned()
    }
    fn put(&self, key: &str, value: &str) {
        self.0
            .lock()
            .expect("secrets lock")
            .insert(key.to_string(), value.to_string());
    }
    fn forget(&self, key: &str) {
        self.0.lock().expect("secrets lock").remove(key);
    }
}

pub async fn authenticate(
    handle: &mut Handle<Client>,
    target: &Endpoint,
    login: &Login,
    asker: &UnboundedSender<Question>,
    secrets: &Secrets,
) -> Result<()> {
    match login {
        Login::Key(path) => {
            if !with_key(handle, target, path, asker, secrets).await? {
                bail!(
                    "{} refused the key {} for user {}",
                    target.name,
                    path.display(),
                    target.user
                );
            }
            Ok(())
        }
        Login::Password(password) => {
            let result = handle.authenticate_password(&target.user, password).await?;
            if !result.success() {
                bail!(
                    "{} refused the password for user {}",
                    target.name,
                    target.user
                );
            }
            Ok(())
        }
        Login::Auto => {
            // Like ssh: the alias's key first, a password if that does not work.
            let mut note = String::new();
            if let Some(identity) = &target.identity {
                match with_key(handle, target, identity, asker, secrets).await {
                    Ok(true) => return Ok(()),
                    Ok(false) => note = format!(" (key {} was refused)", identity.display()),
                    Err(err) => note = format!(" (key {}: {err})", identity.display()),
                }
            }
            with_password_prompt(handle, target, &note, asker, secrets).await
        }
    }
}

/// Offers the key at `path`; `Ok(false)` when the server refuses it.
async fn with_key(
    handle: &mut Handle<Client>,
    target: &Endpoint,
    path: &Path,
    asker: &UnboundedSender<Question>,
    secrets: &Secrets,
) -> Result<bool> {
    let key = load_key(path, asker, secrets).await?;
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    let result = handle
        .authenticate_publickey(
            &target.user,
            PrivateKeyWithHashAlg::new(Arc::new(key), hash),
        )
        .await?;
    Ok(result.success())
}

async fn with_password_prompt(
    handle: &mut Handle<Client>,
    target: &Endpoint,
    note: &str,
    asker: &UnboundedSender<Question>,
    secrets: &Secrets,
) -> Result<()> {
    let cache_key = format!("password:{}", target.label());
    for attempt in 1..=ATTEMPTS {
        let password = match secrets.get(&cache_key) {
            Some(password) => password,
            None => {
                let note = if attempt > 1 {
                    " (wrong, try again)"
                } else {
                    note
                };
                let Some(password) = ask(
                    asker,
                    "SSH password",
                    format!("Password for {}{note}", target.label()),
                )
                .await
                else {
                    bail!("login to {} cancelled", target.label());
                };
                password
            }
        };
        let result = handle
            .authenticate_password(&target.user, &password)
            .await?;
        if result.success() {
            secrets.put(&cache_key, &password);
            return Ok(());
        }
        secrets.forget(&cache_key);
    }
    bail!(
        "{} refused the password for user {} {ATTEMPTS} times",
        target.name,
        target.user
    )
}

async fn load_key(
    path: &Path,
    asker: &UnboundedSender<Question>,
    secrets: &Secrets,
) -> Result<PrivateKey> {
    match load_secret_key(path, None) {
        Ok(key) => return Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => {}
        Err(err) => bail!("reading the key {}: {err}", path.display()),
    }
    let cache_key = format!("passphrase:{}", path.display());
    for attempt in 1..=ATTEMPTS {
        let passphrase = match secrets.get(&cache_key) {
            Some(passphrase) => passphrase,
            None => {
                let note = if attempt > 1 {
                    " (wrong, try again)"
                } else {
                    ""
                };
                let Some(passphrase) = ask(
                    asker,
                    "Key passphrase",
                    format!("Passphrase for {}{note}", path.display()),
                )
                .await
                else {
                    bail!("unlocking {} cancelled", path.display());
                };
                passphrase
            }
        };
        if let Ok(key) = load_secret_key(path, Some(&passphrase)) {
            secrets.put(&cache_key, &passphrase);
            return Ok(key);
        }
        secrets.forget(&cache_key);
    }
    bail!("wrong passphrase for {} {ATTEMPTS} times", path.display())
}

async fn ask(asker: &UnboundedSender<Question>, title: &str, prompt: String) -> Option<String> {
    let (reply, answer) = oneshot::channel();
    asker
        .send(Question::Secret {
            title: title.to_string(),
            prompt,
            reply,
        })
        .ok()?;
    answer.await.ok().flatten()
}
