//! Live service logs followed by the agent and sent as notifications.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::connection::Handle;
use shipdeck_protocol::message::agent::{
    AgentLogLine, AgentLogsEnded, AgentLogsFollow, AgentLogsStop, AgentLogsView, AgentManager,
    LOGS_ENDED, LOGS_LINE,
};
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

/// The longest log line kept.
const MAX_LINE: usize = 8 * 1024;

/// The followed logs of one agent connection.
#[derive(Clone, Default)]
pub(crate) struct Logs {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    running: HashMap<u64, oneshot::Sender<()>>,
}

impl Logs {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts following the service's log; its lines go to `connection`.
    pub(crate) fn follow(
        &self,
        connection: &Handle,
        request: &AgentLogsFollow,
    ) -> Result<AgentLogsView> {
        if !(1..=1000).contains(&request.lines) {
            bail!("log lines must be between 1 and 1000");
        }
        let command = command(request)?;
        let (stop, stopped) = oneshot::channel();
        let view = {
            let mut inner = self.lock();
            inner.next = inner
                .next
                .checked_add(1)
                .context("log view ids exhausted")?;
            let view = inner.next;
            inner.running.insert(view, stop);
            view
        };
        let logs = self.clone();
        let connection = connection.clone();
        tokio::spawn(async move {
            let result = follow(command, view, &connection, stopped).await;
            let (code, error) = match result {
                Ok(code) => (code, None),
                Err(err) => (None, Some(format!("{err:#}"))),
            };
            let _ = connection.notify(LOGS_ENDED, &AgentLogsEnded { view, code, error });
            logs.lock().running.remove(&view);
        });
        Ok(AgentLogsView { view })
    }

    /// Stops a log; its task sends the end notification.
    pub(crate) fn stop(&self, request: AgentLogsStop) {
        let sender = self.lock().running.remove(&request.view);
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

/// The command that prints the service's log, then follows it.
fn command(request: &AgentLogsFollow) -> Result<tokio::process::Command> {
    let service = &request.service;
    if !crate::service::safe_service(&service.service) {
        bail!("invalid service name");
    }
    let mut command = match service.manager {
        AgentManager::Systemd => {
            let mut command = if crate::service::is_root() {
                tokio::process::Command::new("journalctl")
            } else {
                let mut command = tokio::process::Command::new("sudo");
                command.args(["-n", "journalctl"]);
                command
            };
            command
                .arg("-u")
                .arg(&service.service)
                .args(["-f", "-n"])
                .arg(request.lines.to_string())
                .arg("--no-pager");
            command
        }
        AgentManager::Pm2 => {
            if service.user.is_empty() {
                bail!("the request has no execution user for PM2");
            }
            let login = crate::identity::resolve(&service.user)?;
            let program = login.pm2.clone().unwrap_or_else(|| "pm2".to_string());
            let lines = request.lines.to_string();
            let std_command = crate::identity::command(
                &login,
                &program,
                &["logs", &service.service, "--raw", "--lines", &lines],
                &std::collections::BTreeMap::new(),
            );
            tokio::process::Command::from(std_command)
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

async fn follow(
    mut command: tokio::process::Command,
    view: u64,
    connection: &Handle,
    mut stopped: oneshot::Receiver<()>,
) -> Result<Option<u32>> {
    let mut child = command.spawn().context("starting the log command")?;
    let stdout = child.stdout.take().context("the log has no stdout")?;
    let stderr = child.stderr.take().context("the log has no stderr")?;
    let out = tokio::spawn(read_lines(stdout, view, connection.clone()));
    let err = tokio::spawn(read_lines(stderr, view, connection.clone()));
    let mut was_stopped = false;
    let status = tokio::select! {
        _ = &mut stopped => {
            was_stopped = true;
            let _ = child.start_kill();
            child.wait().await.ok()
        }
        status = child.wait() => status.ok(),
    };
    if was_stopped {
        // A pump that ignores the closed pipe must not hold the log open.
        out.abort();
        err.abort();
    } else {
        let _ = out.await;
        let _ = err.await;
    }
    Ok(status
        .and_then(|status| status.code())
        .and_then(|code| u32::try_from(code).ok()))
}

async fn read_lines<R>(mut reader: R, view: u64, connection: Handle)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut chunk = [0_u8; 8 * 1024];
    let mut line = LineBuffer::default();
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => {
                for byte in &chunk[..count] {
                    if let Some(text) = line.push(*byte)
                        && !send_line(&connection, view, text)
                    {
                        return;
                    }
                }
            }
            Err(_) => return,
        }
    }
    if let Some(text) = line.finish() {
        let _ = send_line(&connection, view, text);
    }
}

fn send_line(connection: &Handle, view: u64, line: String) -> bool {
    connection
        .notify(LOGS_LINE, &AgentLogLine { view, line })
        .is_ok()
}

/// Splits a stream into lines while keeping at most [`MAX_LINE`] bytes of
/// the line being read, so one huge service line cannot grow memory.
#[derive(Default)]
struct LineBuffer {
    bytes: Vec<u8>,
    dropping: bool,
}

impl LineBuffer {
    fn push(&mut self, byte: u8) -> Option<String> {
        if byte == b'\n' {
            let line = String::from_utf8_lossy(&self.bytes).into_owned();
            self.bytes.clear();
            self.dropping = false;
            return Some(line);
        }
        if self.dropping {
            return None;
        }
        if self.bytes.len() >= MAX_LINE {
            self.dropping = true;
            return None;
        }
        self.bytes.push(byte);
        None
    }

    fn finish(&mut self) -> Option<String> {
        if self.bytes.is_empty() {
            return None;
        }
        let line = String::from_utf8_lossy(&self.bytes).into_owned();
        self.bytes.clear();
        self.dropping = false;
        Some(line)
    }
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
