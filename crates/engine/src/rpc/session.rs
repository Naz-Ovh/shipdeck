//! What one app has open: its project, its running operation and its live
//! logs. Every request changes what is here, and nothing else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::oneshot;

use shipdeck_protocol::connection::Handle;
use shipdeck_protocol::message::request::{WorkspaceOpen, WorkspaceOpenResult};
use shipdeck_protocol::wire::{ErrorObject, Id, code};

use crate::config::{self, Deployment, Env, ServerInputs};
use crate::deploy::report::OpId;
use crate::remote::Server;
use crate::ssh::{PASSPHRASE_ENV, Pool};

use super::convert;
use super::pump;

/// One connection's state.
pub struct Session {
    connection: Handle,
    state: Mutex<State>,
    next_op: AtomicU64,
    next_view: AtomicU64,
}

struct State {
    workspace: Option<Workspace>,
    /// The deploy or rollback that holds the engine, until its last event.
    operation: Option<OpId>,
    /// How to stop each live log, by view.
    views: BTreeMap<u64, oneshot::Sender<()>>,
}

struct Workspace {
    /// By slug, which is the deploy name every other message uses.
    deployables: BTreeMap<String, Deployment>,
    pool: Arc<Pool>,
}

impl Session {
    pub fn new(connection: Handle) -> Self {
        Self {
            connection,
            state: Mutex::new(State {
                workspace: None,
                operation: None,
                views: BTreeMap::new(),
            }),
            next_op: AtomicU64::new(1),
            next_view: AtomicU64::new(1),
        }
    }

    /// The sending half, for work that reports after its request is answered.
    pub fn connection(&self) -> Handle {
        self.connection.clone()
    }

    /// Writes a method's answer, or the error that replaced it.
    pub fn reply(&self, id: Id, reply: convert::Reply) {
        // The app may be gone by now; then there is nobody left to tell, and
        // the loop ends on its own once the stream closes.
        let _ = match reply {
            Ok(value) => self.connection.answer(id, &value),
            Err(error) => self.connection.answer_error(id, error),
        };
    }

    pub fn reply_error(&self, id: Id, error: ErrorObject) {
        self.reply(id, Err(error));
    }

    /// `workspace/open`: read the root file or the named deploy files, and
    /// keep the pool the app logs in with.
    ///
    /// # Errors
    /// Returns `INVALID_PARAMS` for a folder that is not there, `BUSY` while a
    /// deploy or rollback runs, and `INTERNAL_ERROR` when a file cannot be
    /// read.
    pub async fn open(&self, params: &WorkspaceOpen) -> Result<WorkspaceOpenResult, ErrorObject> {
        let root = PathBuf::from(&params.root);
        if !root.is_dir() {
            return Err(ErrorObject::new(
                code::INVALID_PARAMS,
                format!("{} is not a folder", root.display()),
            ));
        }
        let files: Vec<PathBuf> = params.deploy_files.iter().map(PathBuf::from).collect();
        let named = params.config.as_ref().map(PathBuf::from);
        let env = Env::load(&root, params.env_file.as_deref().map(Path::new))
            .map_err(|err| ErrorObject::new(code::INVALID_PARAMS, format!("{err:#}")))?;
        let inputs = ServerInputs::new(env, params.ssh.clone(), params.run.clone());
        let (found, failures) = config::open_all(&root, named.as_deref(), &files, &inputs);
        let mut problems = Vec::new();
        let mut deployables: BTreeMap<String, Deployment> = BTreeMap::new();
        for deployment in found {
            deployables.insert(deployment.slug.clone(), deployment);
        }
        for failure in &failures {
            problems.extend(convert::problems(failure));
        }
        let answer = WorkspaceOpenResult {
            deployables: deployables.values().map(convert::deployable_info).collect(),
            problems,
        };
        let (questions, inbox) = unbounded_channel();
        let pool = Pool::new(convert::login(params.login.clone()), questions)
            .with_passphrase(passphrase(inputs.env()));
        // The app's flag wins over each deploy file's `server.ssh.shell_env`.
        let pool = if params.no_shell_env {
            pool.without_shell_env()
        } else {
            pool
        };
        let pool = Arc::new(pool);
        let replaced = {
            let mut state = self.state();
            if state.operation.is_some() {
                return Err(busy());
            }
            state
                .workspace
                .replace(Workspace {
                    deployables,
                    pool: Arc::clone(&pool),
                })
                .map(|old| old.pool)
        };
        pump::questions(self.connection(), inbox);
        if let Some(old) = replaced {
            old.close_all().await;
        }
        Ok(answer)
    }

    /// The deploy called `slug`.
    ///
    /// # Errors
    /// Returns `WORKSPACE_NOT_OPEN` before `workspace/open`, and
    /// `UNKNOWN_DEPLOYABLE` for a name that is not in the workspace, and
    /// `DEPLOYABLE_UNAVAILABLE` for a deploy whose variables are not set.
    pub fn deployment(&self, slug: &str) -> Result<Deployment, ErrorObject> {
        let state = self.state();
        let workspace = state.workspace.as_ref().ok_or_else(not_open)?;
        let deployment = workspace
            .deployables
            .get(slug)
            .ok_or_else(|| convert::unknown_deployable(slug))?;
        match &deployment.unavailable {
            Some(reason) => Err(convert::unavailable_deployable(slug, reason)),
            None => Ok(deployment.clone()),
        }
    }

    /// Opens the deployment folder without a process.
    pub async fn storage(&self, deployment: &Deployment) -> Result<Server, ErrorObject> {
        let pool = self.pool()?;
        let session = pool
            .get(&deployment.endpoint)
            .await
            .map_err(|err| convert::failed(&err))?;
        Ok(Server::new(session, deployment.dir().to_string()))
    }

    /// Opens one handle per process app, or the deployment folder for a
    /// static deploy.
    pub async fn servers(&self, deployment: &Deployment) -> Result<Vec<Server>, ErrorObject> {
        let pool = self.pool()?;
        let session = pool
            .get(&deployment.endpoint)
            .await
            .map_err(|err| convert::failed(&err))?;
        let names: Vec<String> = deployment
            .process_apps()
            .map(|(name, _)| name.to_string())
            .collect();
        if names.is_empty() {
            return Ok(vec![Server::new(session, deployment.dir().to_string())]);
        }
        names
            .into_iter()
            .map(|name| {
                Server::for_app(deployment, &name, Arc::clone(&session))
                    .map_err(|err| convert::failed(&err))
            })
            .collect()
    }

    /// The connections of this workspace.
    ///
    /// # Errors
    /// Returns `WORKSPACE_NOT_OPEN` before `workspace/open`.
    pub fn pool(&self) -> Result<Arc<Pool>, ErrorObject> {
        let state = self.state();
        let workspace = state.workspace.as_ref().ok_or_else(not_open)?;
        Ok(Arc::clone(&workspace.pool))
    }

    /// Takes the engine for a deploy or rollback, or says it is taken.
    ///
    /// # Errors
    /// Returns `BUSY` while another operation runs.
    pub fn begin(&self) -> Result<OpId, ErrorObject> {
        let mut state = self.state();
        if state.operation.is_some() {
            return Err(busy());
        }
        let op = self.next_op.fetch_add(1, Ordering::Relaxed);
        state.operation = Some(op);
        Ok(op)
    }

    /// Frees the engine after an operation's last event.
    pub fn end(&self, op: OpId) {
        let mut state = self.state();
        if state.operation == Some(op) {
            state.operation = None;
        }
    }

    /// The id of the next live log.
    pub fn next_view(&self) -> u64 {
        self.next_view.fetch_add(1, Ordering::Relaxed)
    }

    /// Keeps how to stop a live log, so `logs/stop` can use it.
    pub fn hold_view(&self, view: u64, stop: oneshot::Sender<()>) {
        self.state().views.insert(view, stop);
    }

    /// Stops a live log. Stopping one that already ended is not an error.
    pub fn stop_view(&self, view: u64) {
        self.state().views.remove(&view);
    }

    /// Closes every connection, on `shutdown` or once the app is gone.
    pub async fn close(&self) {
        let pool = self
            .state()
            .workspace
            .take()
            .map(|workspace| workspace.pool);
        if let Some(pool) = pool {
            pool.close_all().await;
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `WORKSPACE_NOT_OPEN` for a call that needs the folder first.
fn not_open() -> ErrorObject {
    ErrorObject::new(code::WORKSPACE_NOT_OPEN, "workspace/open has not run yet")
}

/// `BUSY` for a second deploy or rollback.
fn busy() -> ErrorObject {
    ErrorObject::new(code::BUSY, "a deploy or rollback is already running")
}

/// The passphrase the environment or `.env` holds, so nothing is asked for it.
fn passphrase(env: &Env) -> Option<String> {
    env.get(PASSPHRASE_ENV).map(str::to_string)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
