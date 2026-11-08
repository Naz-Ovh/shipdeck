//! Holds the server's project lock for one deployment or rollback.
//!
//! With an agent, the lock is an OS file lock held by the agent for this SSH
//! session. Without one, a shell on a dedicated SSH channel holds the same
//! lock with `flock`; a monitor watches that channel so a plain SSH deploy
//! notices when its lock is gone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use shipdeck_protocol::message::agent::{AgentOk, LOCK_PROJECT, ProjectLock, UNLOCK_PROJECT};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::Agent;
use crate::remote::Server;
use crate::ssh::{Session, sh_quote};

/// Relaxed ordering is enough: tokens only need uniqueness, not memory ordering.
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// How long the shell may take to take the lock and answer.
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the lock shell may take to end when released.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// The line the locking shell prints once it holds the lock.
const READY: &str = "shipdeck-lock-ok";

/// One channel that both reads and writes, so it can be boxed as one value.
trait LockStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> LockStream for T {}

type LegacyStream = Box<dyn LockStream + Unpin>;

/// A project lock; disconnect also releases it on the server.
pub struct ProjectGuard {
    held: Option<Held>,
    root: String,
    released: bool,
}

enum Held {
    Agent {
        agent: Arc<Agent>,
        session: Arc<Session>,
        token: u64,
    },
    Legacy {
        session: Arc<Session>,
        lock: LegacyLock,
    },
}

/// The shell holding the lock and the reader that notices when it ends.
struct LegacyLock {
    writer: WriteHalf<LegacyStream>,
    alive: Arc<AtomicBool>,
    monitor: JoinHandle<()>,
}

impl ProjectGuard {
    /// Locks one project for the life of this guard, with the agent when it
    /// is there and with `flock` on the plain SSH path.
    ///
    /// # Errors
    /// Returns an error if another session holds the lock or the lock cannot
    /// be taken.
    pub async fn acquire(server: &Server) -> Result<Self> {
        if server.command_lease_active().await? {
            bail!("a server command is still running for this project; wait for it to end");
        }
        let session = server.session_handle();
        let mut last = None;
        let mut held = None;
        for attempt in 0..3 {
            let result = if let Some(agent) = server.session().agent() {
                acquire_agent(&agent, server.dir())
                    .await
                    .map(|(agent, token)| Held::Agent {
                        agent,
                        session: Arc::clone(&session),
                        token,
                    })
            } else {
                acquire_legacy(&session, server.dir())
                    .await
                    .map(|lock| Held::Legacy {
                        session: Arc::clone(&session),
                        lock,
                    })
            };
            match result {
                Ok(found) => {
                    held = Some(found);
                    break;
                }
                Err(err) => {
                    last = Some(err);
                    if attempt < 2 {
                        // A session that just left can hold its lock for a
                        // moment; a real one holds it far longer.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        }
        let Some(held) = held else {
            return Err(last.unwrap_or_else(|| anyhow!("the project lock could not be taken")));
        };
        Ok(Self {
            held: Some(held),
            root: server.dir().to_string(),
            released: false,
        })
    }

    /// Whether the connection or shell that holds the lock is still alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        match self.held.as_ref() {
            Some(Held::Agent { agent, session, .. }) => {
                !session.is_closed() && !agent.task.is_finished()
            }
            Some(Held::Legacy { session, lock }) => {
                !session.is_closed() && lock.alive.load(Ordering::Acquire)
            }
            None => true,
        }
    }

    /// Releases the lock before reporting the operation complete.
    ///
    /// # Errors
    /// Returns an error if the agent cannot confirm release, or the plain SSH
    /// lock shell does not end.
    pub async fn release(mut self) -> Result<()> {
        match self.held.take() {
            Some(Held::Agent { agent, token, .. }) => {
                let _: AgentOk = agent
                    .handle
                    .request(
                        UNLOCK_PROJECT,
                        &ProjectLock {
                            root: self.root.clone(),
                            token,
                        },
                    )
                    .await
                    .context("releasing the project lock")?;
                agent.set_lock_token(None);
            }
            Some(Held::Legacy { mut lock, .. }) => {
                lock.writer
                    .shutdown()
                    .await
                    .context("ending the project lock's input")?;
                match timeout(RELEASE_TIMEOUT, &mut lock.monitor).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        return Err(anyhow!("the project lock monitor stopped: {err}"));
                    }
                    Err(_) => {
                        lock.monitor.abort();
                        bail!("the project lock channel did not end");
                    }
                }
                if lock.alive.load(Ordering::Acquire) {
                    bail!("the project lock channel is still open");
                }
            }
            None => {}
        }
        self.released = true;
        Ok(())
    }
}

async fn acquire_agent(agent: &Arc<Agent>, root: &str) -> Result<(Arc<Agent>, u64)> {
    let token = NEXT_TOKEN
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| anyhow!("project lock tokens exhausted"))?;
    agent
        .handle
        .request::<_, AgentOk>(
            LOCK_PROJECT,
            &ProjectLock {
                root: root.to_string(),
                token,
            },
        )
        .await
        .context("locking the project")?;
    agent.set_lock_token(Some(token));
    Ok((Arc::clone(agent), token))
}

/// Runs a shell on its own SSH channel that holds `flock` until the channel
/// closes. The shell answers once the lock is held, then waits for EOF.
async fn acquire_legacy(session: &Arc<Session>, root: &str) -> Result<LegacyLock> {
    let script = format!(
        "cd {root} || {{ printf 'shipdeck-lock-error: the project folder is missing\\n'; exit 1; }}
command -v flock >/dev/null 2>&1 || {{ printf 'shipdeck-lock-error: flock is required on this server\\n'; exit 1; }}
exec 9>>.shipdeck.lock
if ! flock -n 9; then printf 'shipdeck-lock-error: another operation is using this project\\n'; exit 1; fi
printf 'shipdeck-lock-ok\\n'
cat >/dev/null",
        root = sh_quote(root)
    );
    let command = format!("sh -c {}", sh_quote(&script));
    let mut stream = session.open_stream(&command).await?;
    let mut line = String::new();
    {
        let mut reader = tokio::io::BufReader::new(&mut stream);
        timeout(LOCK_TIMEOUT, reader.read_line(&mut line))
            .await
            .context("the project lock did not answer in time")?
            .context("taking the project lock")?;
    }
    if line.trim_end() != READY {
        let why = line.trim();
        if let Some(why) = why.strip_prefix("shipdeck-lock-error: ") {
            bail!("{why}");
        }
        bail!("the project lock was refused");
    }
    let stream: LegacyStream = Box::new(stream);
    let (reader, writer) = tokio::io::split(stream);
    let alive = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&alive);
    let monitor = tokio::spawn(async move {
        let mut reader = reader;
        let mut buffer = [0_u8; 64];
        while let Ok(count) = reader.read(&mut buffer).await {
            if count == 0 {
                break;
            }
        }
        flag.store(false, Ordering::Release);
    });
    Ok(LegacyLock {
        writer,
        alive,
        monitor,
    })
}

impl Drop for ProjectGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        match self.held.take() {
            Some(Held::Agent { agent, token, .. }) => {
                agent.set_lock_token(None);
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    let root = self.root.clone();
                    runtime.spawn(async move {
                        if let Err(err) = agent
                            .handle
                            .request::<_, AgentOk>(UNLOCK_PROJECT, &ProjectLock { root, token })
                            .await
                        {
                            eprintln!("shipdeck: could not release project lock: {err}");
                        }
                    });
                }
            }
            Some(Held::Legacy { lock, .. }) => {
                let LegacyLock {
                    writer, monitor, ..
                } = lock;
                monitor.abort();
                drop(writer);
            }
            None => {}
        }
    }
}
