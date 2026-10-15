//! Logging in with a key file, a key's text (a CI secret), a password from
//! the command line or SHIPDECK_PASSWORD, or a password asked for in the TUI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use russh::client::Handle;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, decode_secret_key};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::Endpoint;
use super::handler::Client;

const ATTEMPTS: usize = 3;

/// The passphrase given up front, tried before asking.
const GIVEN_PASSPHRASE: &str = "passphrase:given";

/// How to log in, from the command-line flags.
#[derive(Clone)]
pub enum Login {
    Key(PathBuf),
    /// A private key's text, such as a CI secret.
    KeyText(String),
    Password(String),
    /// The `IdentityFile` from `~/.ssh/config` if the host has one, then a
    /// password asked in the TUI.
    Auto,
}

impl Login {
    pub fn describe(&self) -> String {
        match self {
            Login::Key(path) => format!("key {}", path.display()),
            Login::KeyText(_) => "key text".into(),
            Login::Password(_) => "password".into(),
            Login::Auto => "~/.ssh/config key, else a password".into(),
        }
    }
}

/// Never prints a key or a password.
impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

/// A private key's text, and how messages name it.
struct KeySource {
    label: String,
    text: String,
}

impl KeySource {
    fn file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| anyhow::anyhow!("reading the key {}: {err}", path.display()))?;
        Ok(Self {
            label: path.display().to_string(),
            text,
        })
    }

    fn text(text: &str) -> Self {
        Self {
            label: "the key text".into(),
            text: text.to_string(),
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
    /// A passphrase known before any key is read, so nothing is asked.
    pub(super) fn give_passphrase(&self, passphrase: &str) {
        self.put(GIVEN_PASSPHRASE, passphrase);
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
            let key = KeySource::file(path)?;
            if !with_key(handle, target, &key, asker, secrets).await? {
                bail!(
                    "{} refused the key {} for user {}",
                    target.name,
                    path.display(),
                    target.user
                );
            }
            Ok(())
        }
        Login::KeyText(text) => {
            let key = KeySource::text(text);
            if !with_key(handle, target, &key, asker, secrets).await? {
                bail!(
                    "{} refused the key text for user {}",
                    target.name,
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
                let tried = match KeySource::file(identity) {
                    Ok(key) => with_key(handle, target, &key, asker, secrets).await,
                    Err(err) => Err(err),
                };
                match tried {
                    Ok(true) => return Ok(()),
                    Ok(false) => note = format!(" (key {} was refused)", identity.display()),
                    Err(err) => note = format!(" (key {}: {err})", identity.display()),
                }
            }
            with_password_prompt(handle, target, &note, asker, secrets).await
        }
    }
}

/// Offers `key`; `Ok(false)` when the server refuses it.
async fn with_key(
    handle: &mut Handle<Client>,
    target: &Endpoint,
    key: &KeySource,
    asker: &UnboundedSender<Question>,
    secrets: &Secrets,
) -> Result<bool> {
    let key = load_key(key, asker, secrets).await?;
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
    key: &KeySource,
    asker: &UnboundedSender<Question>,
    secrets: &Secrets,
) -> Result<PrivateKey> {
    let label = &key.label;
    match decode_secret_key(&key.text, None) {
        Ok(key) => return Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => {}
        Err(err) => bail!("reading the key {label}: {err}"),
    }
    let cache_key = format!("passphrase:{label}");
    if let Some(given) = secrets.get(GIVEN_PASSPHRASE) {
        if let Ok(found) = decode_secret_key(&key.text, Some(&given)) {
            return Ok(found);
        }
        bail!("the given passphrase does not unlock {label}");
    }
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
                    format!("Passphrase for {label}{note}"),
                )
                .await
                else {
                    bail!("unlocking {label} cancelled");
                };
                passphrase
            }
        };
        if let Ok(found) = decode_secret_key(&key.text, Some(&passphrase)) {
            secrets.put(&cache_key, &passphrase);
            return Ok(found);
        }
        secrets.forget(&cache_key);
    }
    bail!("wrong passphrase for {label} {ATTEMPTS} times")
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

#[cfg(test)]
#[path = "login_test.rs"]
mod tests;
