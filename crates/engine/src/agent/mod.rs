//! The agent on a server: pick its build, upload it, start it over SSH, and
//! keep the JSON-RPC channel open while the session lives.

mod binary;

pub use binary::target_for;

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use shipdeck_protocol::connection::{Connection, Handle};
use shipdeck_protocol::message::agent::{
    AgentHello, AgentHelloResult, AgentShutdown, HELLO, PROTOCOL, SHUTDOWN,
};
use tokio::task::AbortHandle;

use crate::ssh::{Output, Session, sh_quote};

/// A slow upload or a stalled server must not hold the app for ever.
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);

/// One running agent, as the rest of the engine sees it.
pub struct Agent {
    handle: Handle,
    /// Aborting it drops the connection, which closes the SSH channel.
    task: AbortHandle,
    version: String,
    os: String,
    arch: String,
}

impl Agent {
    /// Picks the build, installs it on the server, starts it, and says hello.
    ///
    /// # Errors
    ///
    /// Fails when the server's C library or CPU has no build, the binary
    /// cannot be fetched, the server cannot run it, or it does not answer.
    pub async fn start(session: &Session) -> Result<Self> {
        let target = target_for(session).await?;
        let home = home(session).await?;
        let folder = format!("{home}/.shipdeck/bin");
        let version = env!("CARGO_PKG_VERSION");
        let path = install(session, &folder, target, version).await?;

        let log = format!(
            "{}/agent.log",
            path.rsplit_once('/')
                .map_or(folder.as_str(), |(folder, _)| folder)
        );
        let command = format!(
            "sh -c {}",
            sh_quote(&format!("exec {} 2>>{}", sh_quote(&path), sh_quote(&log)))
        );
        let stream = session.open_stream(&command).await?;
        let (read, write) = tokio::io::split(stream);
        let connection = Connection::start(read, write);
        let handle = connection.handle();
        let task = tokio::spawn(async move {
            let mut connection = connection;
            while connection.recv().await.is_some() {}
        });

        let mut agent = Self {
            handle,
            task: task.abort_handle(),
            version: String::new(),
            os: String::new(),
            arch: String::new(),
        };
        let answer = tokio::time::timeout(HELLO_TIMEOUT, agent.hello())
            .await
            .with_context(|| format!("the agent at {path} did not answer"))??;
        if answer.protocol != PROTOCOL {
            bail!(
                "the agent at {path} speaks protocol {}, and shipdeck speaks {PROTOCOL}",
                answer.protocol
            );
        }
        if answer.version != version {
            bail!(
                "the agent at {path} is version {}, and shipdeck is {version}",
                answer.version
            );
        }
        agent.version = answer.version;
        agent.os = answer.os;
        agent.arch = answer.arch;
        Ok(agent)
    }

    async fn hello(&self) -> Result<AgentHelloResult> {
        self.handle
            .request(
                HELLO,
                &AgentHello {
                    protocol: PROTOCOL,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            )
            .await
            .map_err(|err| anyhow!("the agent refused the hello: {err}"))
    }

    /// Asks the agent to exit, then closes the channel. A gone or silent
    /// agent must not hold the app's exit.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            self.handle
                .request::<_, AgentShutdown>(SHUTDOWN, &AgentShutdown {}),
        )
        .await;
        self.task.abort();
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn os(&self) -> &str {
        &self.os
    }

    #[must_use]
    pub fn arch(&self) -> &str {
        &self.arch
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// `$HOME`, as an absolute path.
async fn home(session: &Session) -> Result<String> {
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

/// Makes sure the agent in `folder` exists and runs; returns the path it
/// started from. A home folder that is `noexec` or not writable falls back to
/// a private folder in tmp.
async fn install(session: &Session, folder: &str, target: &str, version: &str) -> Result<String> {
    let path = format!("{folder}/shipdeck-agent-{version}");
    if probe(session, &path, version).await.is_ok() {
        return Ok(path);
    }
    let home_error = match place(session, &path, target, version, false).await {
        Ok(()) => {
            remove_older(session, folder, version).await;
            return Ok(path);
        }
        Err(err) => format!("{err:#}"),
    };

    let folder = temp_folder(session).await?;
    let path = format!("{folder}/shipdeck-agent-{version}");
    place(session, &path, target, version, true)
        .await
        .with_context(|| {
            format!("the agent cannot run in the home folder ({home_error}) or in {folder}")
        })?;
    Ok(path)
}

/// Downloads and runs the agent at `path`. The server fetches it over HTTPS;
/// nothing is uploaded through SSH.
async fn place(
    session: &Session,
    path: &str,
    target: &str,
    version: &str,
    private: bool,
) -> Result<()> {
    if private {
        let folder = path.rsplit_once('/').map_or(path, |(folder, _)| folder);
        session
            .exec_raw(&format!(
                "mkdir -p -m 0700 {folder} && chmod 0700 {folder}",
                folder = sh_quote(folder)
            ))
            .await?
            .check("making the agent's folder private")?;
    }
    let script = binary::install_script(target, path);
    session
        .exec_raw(&script)
        .await?
        .check("downloading the agent")?;
    probe(session, path, version).await
}

/// `${TMPDIR:-/tmp}/.shipdeck-<uid>`, where an executable may still run.
async fn temp_folder(session: &Session) -> Result<String> {
    let base = session
        .exec_raw("printf '%s' \"${TMPDIR:-/tmp}\"")
        .await
        .context("asking the server for its temporary folder")?
        .stdout;
    let base = base.trim();
    if !base.starts_with('/') {
        bail!("the server did not say where its temporary folder is");
    }
    let uid = session
        .exec_raw("id -u")
        .await
        .context("asking the server for the user id")?
        .stdout;
    let uid = uid.trim();
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("the server did not say the user id");
    }
    Ok(format!("{base}/.shipdeck-{uid}"))
}

/// Runs `<path> --version` and checks the version it prints.
async fn probe(session: &Session, path: &str, version: &str) -> Result<()> {
    let output = session
        .exec_raw(&format!("{} --version 2>/dev/null", sh_quote(path)))
        .await?;
    check_probe(&output, version).with_context(|| format!("running the agent at {path}"))
}

fn check_probe(output: &Output, version: &str) -> Result<()> {
    if !output.success() {
        bail!("it exited with code {}", output.code);
    }
    if !output.stdout.contains(version) {
        bail!("it printed {:?}", output.stdout.trim());
    }
    Ok(())
}

/// Removes older versions in `folder`; a failure is reported, not fatal.
async fn remove_older(session: &Session, folder: &str, version: &str) {
    let keep = format!("shipdeck-agent-{version}");
    let script = format!(
        "find {} -maxdepth 1 -type f -name 'shipdeck-agent-*' ! -name {} -delete",
        sh_quote(folder),
        sh_quote(&keep)
    );
    match session.exec_raw(&script).await {
        Ok(output) if !output.success() => eprintln!(
            "shipdeck: {}: could not remove older agents: {}",
            session.label,
            output.stderr.trim()
        ),
        Ok(_) => {}
        Err(err) => eprintln!(
            "shipdeck: {}: could not remove older agents: {err:#}",
            session.label
        ),
    }
}
