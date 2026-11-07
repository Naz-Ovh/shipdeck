//! One running agent: start it, say hello, and close it with the session.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use shipdeck_protocol::connection::{Connection, Handle};
use shipdeck_protocol::message::agent::{
    AgentHello, AgentHelloResult, AgentShutdown, HELLO, PROTOCOL, SHUTDOWN,
};
use tokio::task::AbortHandle;

use super::{binary, install};
use crate::ssh::{Session, sh_quote};

/// The hello must not hold the app when the agent is already dead.
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
        let target = binary::target_for(session).await?;
        let home = install::home(session).await?;
        let folder = format!("{home}/.shipdeck/bin");
        let version = env!("CARGO_PKG_VERSION");
        let path = install::install(session, &folder, target, version).await?;

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
