//! SSH: one session per `user@host:port`, opened once and reused for every
//! command and upload.

mod endpoint;
mod handler;
mod login;
pub mod ssh_config;

pub use endpoint::Endpoint;
pub use login::{Login, Question};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use russh::client::{self, Handle};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;

use handler::Client;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const UPLOAD_CHUNK: usize = 256 * 1024;

/// Which output stream a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

#[derive(Debug)]
pub struct Output {
    pub code: u32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == 0
    }

    /// Turns a non-zero exit into an error that carries the command's own words.
    pub fn check(self, what: &str) -> Result<Self> {
        if self.success() {
            return Ok(self);
        }
        let detail = [self.stderr.trim(), self.stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("no output");
        bail!("{what} failed (exit {}): {detail}", self.code)
    }
}

pub struct Session {
    handle: Handle<Client>,
    pub label: String,
    /// Who it is logged in as.
    pub user: String,
}

impl Session {
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Runs a POSIX `sh` script and collects its output.
    pub async fn exec(&self, script: &str) -> Result<Output> {
        self.exec_with(script, |_, _| {}).await
    }

    /// Runs a POSIX `sh` script, handing each output line to `on_line` as it
    /// arrives. The script goes through `sh -c` so the login shell (bash,
    /// zsh, fish, ...) does not matter.
    pub async fn exec_with(
        &self,
        script: &str,
        mut on_line: impl FnMut(Stream, &str),
    ) -> Result<Output> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .with_context(|| format!("opening a channel to {}", self.label))?;
        channel
            .exec(true, format!("sh -c {}", sh_quote(script)))
            .await
            .context("starting the remote command")?;

        let mut out = Lines::default();
        let mut err = Lines::default();
        let mut code = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.feed(&data, |l| on_line(Stream::Out, l)),
                ChannelMsg::ExtendedData { data, ext: 1 } => {
                    err.feed(&data, |l| on_line(Stream::Err, l))
                }
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                ChannelMsg::ExitSignal { signal_name, .. } => {
                    bail!("remote command was killed by {signal_name:?}")
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        out.flush(|l| on_line(Stream::Out, l));
        err.flush(|l| on_line(Stream::Err, l));
        Ok(Output {
            code: code.ok_or_else(|| anyhow!("connection to {} closed mid-command", self.label))?,
            stdout: out.all,
            stderr: err.all,
        })
    }

    /// Writes `data` to `remote_path`, over SFTP when the server offers it
    /// and through `cat` otherwise. `progress(sent, total)` follows along.
    pub async fn upload(
        &self,
        data: &[u8],
        remote_path: &str,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        match self.sftp().await {
            Ok(sftp) => {
                let mut file = sftp
                    .create(remote_path)
                    .await
                    .with_context(|| format!("creating {remote_path} over SFTP"))?;
                let total = data.len() as u64;
                let mut sent = 0u64;
                progress(0, total);
                for chunk in data.chunks(UPLOAD_CHUNK) {
                    file.write_all(chunk).await.context("uploading")?;
                    sent += chunk.len() as u64;
                    progress(sent, total);
                }
                file.shutdown().await.context("finishing the upload")?;
                let _ = sftp.close().await;
                Ok(())
            }
            Err(_) => self.upload_with_cat(data, remote_path, progress).await,
        }
    }

    async fn sftp(&self) -> Result<SftpSession> {
        let channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let config = russh_sftp::client::Config {
            max_concurrent_writes: 64,
            request_timeout_secs: 60,
            ..Default::default()
        };
        Ok(SftpSession::new_with_config(channel.into_stream(), config).await?)
    }

    async fn upload_with_cat(
        &self,
        data: &[u8],
        remote_path: &str,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        let mut channel = self.handle.channel_open_session().await?;
        channel
            .exec(
                true,
                format!(
                    "sh -c {}",
                    sh_quote(&format!("cat > {}", sh_quote(remote_path)))
                ),
            )
            .await?;
        let total = data.len() as u64;
        let mut sent = 0u64;
        progress(0, total);
        for chunk in data.chunks(UPLOAD_CHUNK) {
            channel.data(chunk).await.context("uploading")?;
            sent += chunk.len() as u64;
            progress(sent, total);
        }
        channel.eof().await?;
        let mut code = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        match code {
            Some(0) => Ok(()),
            Some(code) => bail!("writing {remote_path} failed (exit {code})"),
            None => bail!("connection closed during the upload"),
        }
    }

    pub async fn close(&self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "bye", "en")
            .await;
    }
}

/// One target's session, or `None` until the first connect. The async lock
/// makes a second caller wait for the first connection instead of opening another.
type Slot = Arc<tokio::sync::Mutex<Option<Arc<Session>>>>;

/// Opens sessions on demand and keeps them for the rest of the run.
pub struct Pool {
    login: Login,
    asker: UnboundedSender<Question>,
    secrets: Arc<login::Secrets>,
    slots: Mutex<HashMap<Endpoint, Slot>>,
}

impl Pool {
    pub fn new(login: Login, asker: UnboundedSender<Question>) -> Self {
        Self {
            login,
            asker,
            secrets: Arc::default(),
            slots: Mutex::default(),
        }
    }

    pub fn login(&self) -> &Login {
        &self.login
    }

    /// The open session for `target`, connecting first if needed. Two callers
    /// for the same target share one connection attempt.
    pub async fn get(&self, target: &Endpoint) -> Result<Arc<Session>> {
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
        let session = Arc::new(self.connect(target).await?);
        *slot = Some(session.clone());
        Ok(session)
    }

    async fn connect(&self, target: &Endpoint) -> Result<Session> {
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
        login::authenticate(&mut handle, target, &self.login, &self.asker, &self.secrets).await?;
        Ok(Session {
            handle,
            label,
            user: target.user.clone(),
        })
    }

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

/// Splits a byte stream into lines, keeping the whole text too.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
    all: String,
}

impl Lines {
    fn feed(&mut self, bytes: &[u8], mut emit: impl FnMut(&str)) {
        self.all.push_str(&String::from_utf8_lossy(bytes));
        for &byte in bytes {
            if byte == b'\n' {
                emit(String::from_utf8_lossy(&self.partial).trim_end());
                self.partial.clear();
            } else {
                self.partial.push(byte);
            }
        }
    }

    fn flush(&mut self, mut emit: impl FnMut(&str)) {
        if !self.partial.is_empty() {
            emit(String::from_utf8_lossy(&self.partial).trim_end());
            self.partial.clear();
        }
    }
}

/// Quotes `text` as one `sh` word.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
#[path = "ssh_test.rs"]
mod tests;
