//! The session pool: one connection per `user@host:port`, opened on demand.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use russh::client;
use tokio::sync::mpsc::UnboundedSender;

use super::endpoint::Endpoint;
use super::handler::Client;
use super::login::{Login, Question};
use super::shell_env::ShellEnv;
use super::{CONNECT_TIMEOUT, Session, login};

/// One target's session, or `None` until the first connect. The async lock
/// makes a second caller wait for the first connection instead of opening another.
type Slot = Arc<tokio::sync::Mutex<Option<Arc<Session>>>>;

/// Opens sessions on demand and keeps them for the rest of the run.
pub struct Pool {
    login: Login,
    asker: UnboundedSender<Question>,
    secrets: Arc<login::Secrets>,
    slots: Mutex<HashMap<Endpoint, Slot>>,
    /// False with `--no-shell-env`, whatever `ssh.shell_env` says.
    shell_env: bool,
}

impl Pool {
    #[must_use]
    pub fn new(login: Login, asker: UnboundedSender<Question>) -> Self {
        Self {
            login,
            asker,
            secrets: Arc::default(),
            slots: Mutex::default(),
            shell_env: true,
        }
    }

    /// Runs commands without the login shell's environment on every server.
    #[must_use]
    pub fn without_shell_env(mut self) -> Self {
        self.shell_env = false;
        self
    }

    /// Unlocks an encrypted key with `passphrase` instead of asking.
    #[must_use]
    pub fn with_passphrase(self, passphrase: Option<String>) -> Self {
        if let Some(passphrase) = passphrase {
            self.secrets.give_passphrase(&passphrase);
        }
        self
    }

    pub fn login(&self) -> &Login {
        &self.login
    }

    /// The open session for `target`, connecting first if needed. Two callers
    /// for the same target share one connection attempt.
    ///
    /// # Errors
    ///
    /// Fails when the server cannot be reached or the login is refused.
    ///
    /// # Panics
    ///
    /// Panics when the pool's slot lock is poisoned.
    pub async fn get(&self, target: &Endpoint) -> Result<Arc<Session>> {
        self.get_with(target, |_| {}, |_, _| {}).await
    }

    /// Opens a connection, naming each slow phase to `status`.
    ///
    /// # Errors
    ///
    /// Fails when the server cannot be reached or the login is refused.
    ///
    /// # Panics
    ///
    /// Panics when the pool's slot lock is poisoned.
    pub async fn get_with<F, G>(
        &self,
        target: &Endpoint,
        status: F,
        bytes: G,
    ) -> Result<Arc<Session>>
    where
        F: Fn(&str) + Sync,
        G: Fn(u64, u64) + Sync,
    {
        let slot = {
            let mut slots = self.slots.lock().expect("pool lock");
            slots.entry(target.clone()).or_default().clone()
        };
        let mut slot = slot.lock().await;
        if let Some(session) = slot.as_ref()
            && !session.is_closed()
        {
            return Ok(session.clone());
        }
        let session = Arc::new(self.connect(target, &status, &bytes).await?);
        *slot = Some(session.clone());
        Ok(session)
    }

    async fn connect(
        &self,
        target: &Endpoint,
        status: &(dyn Fn(&str) + Sync),
        bytes: &(dyn Fn(u64, u64) + Sync),
    ) -> Result<Session> {
        let label = target.label();
        // Errors name the alias and the address it stands for.
        let place = if target.is_alias() {
            format!("{label} ({})", target.address())
        } else {
            label.clone()
        };
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 4,
            ..Default::default()
        });
        let client = Client::new(target.host.clone(), target.port, self.asker.clone());
        let rejected = client.rejection();
        status("opening the SSH connection");
        let connecting = client::connect(config, (target.host.as_str(), target.port), client);
        let mut handle = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Err(_) => bail!(
                "connecting to {place} timed out after {}s",
                CONNECT_TIMEOUT.as_secs()
            ),
            Ok(Err(err)) => {
                if let Some(reason) = rejected.lock().expect("rejection lock").take() {
                    bail!("{reason}");
                }
                return Err(anyhow!(err).context(format!("connecting to {place}")));
            }
            Ok(Ok(handle)) => handle,
        };
        status("logging in");
        login::authenticate(&mut handle, target, &self.login, &self.asker, &self.secrets).await?;
        status("connected");
        let mut session = Session {
            handle,
            label,
            user: target.user.clone(),
            shell_env: ShellEnv::Off,
            prelude: String::new(),
            agent: Mutex::new(None),
        };
        if self.shell_env && target.shell_env {
            status("reading the login shell's environment");
            session.load_shell_env().await;
        }
        session
            .start_agent_with(status, bytes)
            .await
            .context("preparing the shipdeck agent")?;
        Ok(session)
    }

    /// Closes the session for `target`, if one is open; the next `get`
    /// connects again.
    ///
    /// # Panics
    ///
    /// Panics when the pool's slot lock is poisoned.
    pub async fn close(&self, target: &Endpoint) {
        let slot = self.slots.lock().expect("pool lock").get(target).cloned();
        if let Some(slot) = slot
            && let Some(session) = slot.lock().await.take()
        {
            session.close().await;
        }
    }

    /// Closes every open session; the next `get` connects again.
    ///
    /// # Panics
    ///
    /// Panics when the pool's slot lock is poisoned.
    pub async fn close_all(&self) {
        let slots: Vec<_> = self
            .slots
            .lock()
            .expect("pool lock")
            .values()
            .cloned()
            .collect();
        for slot in slots {
            if let Some(session) = slot.lock().await.take() {
                session.close().await;
            }
        }
    }
}
