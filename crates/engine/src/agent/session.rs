//! One running agent: start it, say hello, and close it with the session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use shipdeck_protocol::connection::{Connection, Handle};
use shipdeck_protocol::message::agent::{
    AgentHello, AgentHelloResult, AgentOk, AgentShutdown, HASH_CURRENT, HELLO, HashCurrent,
    HashCurrentResult, INSPECT_WORK, InspectWork, PREPARE_PROJECT, PROTOCOL, SERVER_FACTS,
    SHUTDOWN, SWITCH_RELEASE, ServerFacts, ServerFactsRequest, SwitchRelease, UNPACK_RELEASE,
    UnpackRelease, UnpackResult, WorkInspection,
};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;

use super::version::{Action, Installed, Version, choose};
use super::{binary, development, install, layout};
use crate::ssh::Session;

/// The hello must not hold the app when the agent is already dead.
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);

/// The receivers waiting for one agent's followed logs.
pub(super) type LogViews = Arc<Mutex<HashMap<u64, UnboundedSender<super::ops::AgentLogEvent>>>>;

/// One running agent, as the rest of the engine sees it.
pub struct Agent {
    pub(super) handle: Handle,
    /// Aborting it drops the connection, which closes the SSH channel.
    pub(super) task: AbortHandle,
    version: String,
    os: String,
    arch: String,
    pub(super) path: String,
    /// The sudo prefix and identity used for the receiver channel too.
    pub(super) prefix: String,
    pub(super) operator: String,
    pub(super) operator_home: String,
    pub(super) logs: LogViews,
    /// Set when the reader task sees the agent's stream end.
    done: Arc<Notify>,
    /// The token of the project lock this engine holds through the agent.
    lock_token: Mutex<Option<u64>>,
}

impl Agent {
    /// Checks the installed agent versions, installs the release this
    /// shipdeck needs, starts it as root, and says hello.
    ///
    /// # Errors
    ///
    /// Fails when the account cannot use sudo, a newer agent major is
    /// installed, the release cannot be fetched or verified, the server has
    /// no build, the agent cannot run, or it does not answer.
    pub async fn start(session: &Session) -> Result<Self> {
        let label = env!("CARGO_PKG_VERSION");
        let cli = Version::parse(label)?;
        let prefix = layout::sudo_prefix(session).await?;
        let found = layout::installed(session, &prefix).await?;
        let selected = match choose(&cli, &found) {
            Action::Use(installed) => installed,
            Action::Refuse(installed) => bail!(
                "the installed agent is v{}, but this shipdeck is v{label}. An agent with a \
                 newer major version cannot serve it: this shipdeck requires agent major {}. \
                 Change the agent installation to a v{}.x release.",
                installed.label,
                cli.major,
                cli.major,
            ),
            Action::Replace | Action::Install => {
                let target = binary::target_for(session).await?;
                let path = match std::env::var_os(development::BINARY_ENV) {
                    Some(local) => {
                        development::install(session, &prefix, local.as_ref(), target, label)
                            .await?
                    }
                    None => install::install(session, &prefix, target, label).await?,
                };
                Installed {
                    version: cli,
                    label: label.to_string(),
                    path,
                }
            }
        };

        let home = layout::home(session).await?;
        let command = layout::launch_command(&prefix, &selected.path, &session.user, &home);
        let path = selected.path;
        let stream = session.open_stream(&command).await?;
        let (read, write) = tokio::io::split(stream);
        let connection = Connection::start(read, write);
        let handle = connection.handle();
        let logs: LogViews = Arc::new(Mutex::new(HashMap::new()));
        let watched = Arc::clone(&logs);
        let done = Arc::new(Notify::new());
        let ended = Arc::clone(&done);
        let task = tokio::spawn(async move {
            let mut connection = connection;
            while let Some(incoming) = connection.recv().await {
                if let shipdeck_protocol::connection::Incoming::Other {
                    id: None,
                    method,
                    params,
                } = incoming
                {
                    super::ops::deliver_log(&watched, &method, params);
                }
            }
            ended.notify_one();
        });

        let mut agent = Self {
            handle,
            task: task.abort_handle(),
            version: String::new(),
            os: String::new(),
            arch: String::new(),
            path: path.clone(),
            prefix: prefix.clone(),
            operator: session.user.clone(),
            operator_home: home.clone(),
            logs,
            done,
            lock_token: Mutex::new(None),
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
        let actual = Version::parse(&answer.version)?;
        if actual.major != cli.major {
            bail!(
                "the agent at {path} says it is v{}, but this shipdeck requires agent major {}",
                answer.version,
                cli.major
            );
        }
        if answer.version != selected.label {
            bail!(
                "the agent at {path} is labeled v{}, but it reports v{}",
                selected.label,
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

    /// Hashes files from the live release using the connected agent.
    ///
    /// # Errors
    /// Returns an error if the agent cannot read the files or the channel closes.
    pub async fn hash_current(&self, root: &str, paths: &[String]) -> Result<HashCurrentResult> {
        self.handle
            .request(
                HASH_CURRENT,
                &HashCurrent {
                    root: root.to_string(),
                    paths: paths.to_vec(),
                },
            )
            .await
            .map_err(|err| anyhow!("the agent could not hash current files: {err}"))
    }

    /// Inspects the live link and unfinished release files.
    ///
    /// # Errors
    /// Returns an error if the agent cannot inspect the deployment folder.
    pub async fn inspect_work(&self, root: &str) -> Result<WorkInspection> {
        self.handle
            .request(
                INSPECT_WORK,
                &InspectWork {
                    root: root.to_string(),
                },
            )
            .await
            .context("inspecting unfinished work")
    }

    /// Creates the release, backup, and shared directories.
    ///
    /// # Errors
    /// Returns an error if the agent cannot create a directory.
    pub async fn prepare_project(&self, root: &str) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                PREPARE_PROJECT,
                &InspectWork {
                    root: root.to_string(),
                },
            )
            .await
            .context("preparing project directories")?;
        Ok(())
    }

    /// Atomically points the live link to an existing release.
    ///
    /// # Errors
    /// Returns an error if the switch or its event record fails.
    pub async fn switch_release(&self, root: &str, release: &str) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                SWITCH_RELEASE,
                &SwitchRelease {
                    root: root.to_string(),
                    release: release.to_string(),
                },
            )
            .await
            .context("switching the live release")?;
        Ok(())
    }

    /// Unpacks and verifies a staged release with the agent.
    ///
    /// # Errors
    /// Returns an error if extraction or verification cannot finish.
    pub async fn unpack_release(
        &self,
        root: &str,
        release: &str,
        modes: &[u32],
        max_bytes: u64,
        max_entries: u64,
    ) -> Result<UnpackResult> {
        let request = UnpackRelease {
            root: root.to_string(),
            release: release.to_string(),
            modes: modes.to_vec(),
            max_bytes,
            max_entries,
        };
        self.handle
            .request(UNPACK_RELEASE, &request)
            .await
            .context("unpacking the release with the agent")
    }

    /// Reads current facts from the connected server.
    ///
    /// # Errors
    /// Returns an error when the agent cannot read a system fact.
    pub async fn server_facts(&self, disk_path: &str) -> Result<ServerFacts> {
        self.handle
            .request(
                SERVER_FACTS,
                &ServerFactsRequest {
                    disk_path: disk_path.to_string(),
                },
            )
            .await
            .context("reading server information")
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
        // The answer is written before the agent exits; wait for the stream
        // to end so the project lock it held is free before the next run.
        let _ = tokio::time::timeout(Duration::from_secs(2), self.done.notified()).await;
        self.task.abort();
    }

    /// Remembers or forgets the project lock token this connection holds.
    pub(super) fn set_lock_token(&self, token: Option<u64>) {
        *self
            .lock_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = token;
    }

    /// The token of the project lock this connection holds, if any.
    #[must_use]
    pub(super) fn lock_token(&self) -> Option<u64> {
        *self
            .lock_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
