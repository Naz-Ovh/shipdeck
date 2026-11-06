//! What one app has open: its workspace, its running operation and its live
//! logs. Every request changes what is here, and nothing else.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::oneshot;

use shipdeck_protocol::connection::Handle;
use shipdeck_protocol::message::request::{WorkspaceOpen, WorkspaceOpenResult};
use shipdeck_protocol::wire::{ErrorObject, Id, code};

use crate::config::{self, Deployable};
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
    /// By slug, which is what every other message calls a project.
    deployables: BTreeMap<String, Deployable>,
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

    /// `workspace/open`: scan the folder, and keep the pool the app logs in
    /// with. One engine works on one folder of projects.
    ///
    /// # Errors
    /// Returns `INVALID_PARAMS` for a folder that is not there, `BUSY` while a
    /// deploy or rollback runs, and `INTERNAL_ERROR` when the folder cannot be
    /// read.
    pub async fn open(&self, params: &WorkspaceOpen) -> Result<WorkspaceOpenResult, ErrorObject> {
        let root = PathBuf::from(&params.root);
        if !root.is_dir() {
            return Err(ErrorObject::new(
                code::INVALID_PARAMS,
                format!("{} is not a folder", root.display()),
            ));
        }
        let found = config::discover(&root).map_err(|err| convert::failed(&err))?;
        let answer = WorkspaceOpenResult {
            deployables: found
                .deployables
                .iter()
                .map(convert::deployable_info)
                .collect(),
            problems: found.problems.iter().flat_map(convert::problems).collect(),
        };
        let (questions, inbox) = unbounded_channel();
        let pool = Arc::new(
            Pool::new(convert::login(params.login.clone()), questions)
                .with_passphrase(passphrase()),
        );
        let mut deployables = BTreeMap::new();
        for deployable in found.deployables {
            deployables.insert(deployable.slug.clone(), deployable);
        }
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

    /// The project called `slug`.
    ///
    /// # Errors
    /// Returns `WORKSPACE_NOT_OPEN` before `workspace/open`, and
    /// `UNKNOWN_DEPLOYABLE` for a name that is not in the workspace.
    pub fn deployable(&self, slug: &str) -> Result<Deployable, ErrorObject> {
        let state = self.state();
        let workspace = state.workspace.as_ref().ok_or_else(not_open)?;
        workspace
            .deployables
            .get(slug)
            .cloned()
            .ok_or_else(|| convert::unknown_deployable(slug))
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

    /// A server for `deployable`, connected when it is not connected yet.
    ///
    /// # Errors
    /// Returns `WORKSPACE_NOT_OPEN`, or `INTERNAL_ERROR` when the login fails
    /// or the app cancels a question.
    pub async fn server(&self, deployable: &Deployable) -> Result<Server, ErrorObject> {
        let pool = self.pool()?;
        let session = pool
            .get(&deployable.endpoint)
            .await
            .map_err(|err| convert::failed(&err))?;
        Ok(Server::new(session, &deployable.config.service))
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

/// The passphrase the app put in the environment, so nothing is asked for it.
fn passphrase() -> Option<String> {
    std::env::var(PASSPHRASE_ENV)
        .ok()
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
#[path = "session_test.rs"]
mod tests;
