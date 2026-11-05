//! Talking to the engine: requests out, events and questions in.

use std::collections::HashMap;
use std::fmt::{self, Display};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::message::ask::Ask;
use crate::message::event::Event;
use crate::message::method;
use crate::message::request::{
    ConnectionClose, DeployStart, Done, Initialize, InitializeResult, LogsFollow, LogsFollowResult,
    LogsStop, OperationStarted, ReleaseContents, ReleaseContentsResult, ReleaseReadFile,
    ReleaseReadFileResult, ReleasesList, ReleasesListResult, RollbackStart, ServiceControl,
    StatusGet, StatusGetResult, WorkspaceOpen, WorkspaceOpenResult,
};
use crate::wire::{self, ErrorObject, Id, Notification, Request, Response};

/// Why a call to the engine did not finish.
#[derive(Debug)]
pub enum Error {
    /// The stream could not be read or written.
    Io(std::io::Error),
    /// A message was not valid JSON, or not the shape it should be.
    Json(serde_json::Error),
    /// The engine answered with an error.
    Rpc(ErrorObject),
    /// The stream ended; nothing more will arrive.
    Closed,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "the stream failed: {err}"),
            Self::Json(err) => write!(f, "a message was not what it should be: {err}"),
            Self::Rpc(err) => write!(f, "the engine said no: {err}"),
            Self::Closed => f.write_str("the engine closed the stream"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// What arrived from the engine, other than an answer to a request.
#[derive(Debug)]
pub enum Incoming {
    /// Something the engine reports; nothing to answer.
    Event(Event),
    /// A question, with the id its answer must carry.
    Ask { id: Id, ask: Ask },
    /// A method this version does not know, or whose params do not fit.
    /// An id means the engine waits for an answer.
    Other {
        id: Option<Id>,
        method: String,
        params: Value,
    },
    /// A line that was not a JSON-RPC message this version reads.
    Malformed { id: Option<Id>, error: String },
}

/// The chat with one engine, over a stream that carries one message per line.
///
/// Dropping it closes the client; the engine then takes no new work.
pub struct Client {
    out: mpsc::UnboundedSender<Outbound>,
    shared: Arc<Shared>,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    next: AtomicU64,
    task: JoinHandle<()>,
}

/// One line the client has to write.
enum Outbound {
    Line(String),
}

/// What the reading task and the client share.
struct Shared {
    /// Requests waiting for their answer, by id.
    pending: Mutex<HashMap<Id, oneshot::Sender<Result<Value, Error>>>>,
    /// The way events reach the app. Dropping it ends `recv`, so the reading
    /// task takes it out when the stream ends.
    incoming: Mutex<Option<mpsc::UnboundedSender<Incoming>>>,
}

impl Shared {
    /// The pending map. A panic elsewhere cannot leave it half-written: only
    /// an insert and a remove happen here, both under this lock.
    fn pending(&self) -> MutexGuard<'_, HashMap<Id, oneshot::Sender<Result<Value, Error>>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, incoming: Incoming) {
        let sender = self.incoming.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = sender.as_ref() {
            // The app may be gone; then nothing is left to tell.
            let _ = sender.send(incoming);
        }
    }

    /// Answers the request `id` waits for, if it is still waiting.
    fn finish(&self, id: &Id, result: Result<Value, Error>) {
        if let Some(reply) = self.pending().remove(id) {
            let _ = reply.send(result);
        }
    }

    /// Fails every waiting request, because the stream ended.
    fn close(&self) {
        // No new event can arrive; `recv` ends once the buffered ones are read.
        let sender = self
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(sender);
        let waiting: Vec<_> = self.pending().drain().map(|(_, reply)| reply).collect();
        for reply in waiting {
            let _ = reply.send(Err(Error::Closed));
        }
    }
}

impl Client {
    /// Starts a client on the two halves of one stream: a child process's
    /// pipes, a socket, or a test pipe.
    #[must_use]
    pub fn start<R, W>(read: R, write: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out, outbox) = mpsc::unbounded_channel();
        let (inbox, incoming) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            incoming: Mutex::new(Some(inbox)),
        });
        let reader = Arc::clone(&shared);
        let task = tokio::spawn(run(read, write, outbox, reader));
        Self {
            out,
            shared,
            incoming,
            next: AtomicU64::new(1),
            task,
        }
    }

    /// Sends one request and waits for its answer.
    ///
    /// # Errors
    /// Returns an error when the stream closes, the engine answers with an
    /// error, or the answer is not the expected shape.
    pub async fn request<P, R>(&self, method: &str, params: &P) -> Result<R, Error>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let params = serde_json::to_value(params)?;
        let id = Id::Number(self.next.fetch_add(1, Ordering::Relaxed));
        let (reply, answer) = oneshot::channel();
        self.shared.pending().insert(id.clone(), reply);
        let line = wire::to_line(&Request::new(id.clone(), method, params))?;
        if self.out.send(Outbound::Line(line)).is_err() {
            self.shared.pending().remove(&id);
            return Err(Error::Closed);
        }
        match answer.await {
            Ok(Ok(value)) => Ok(serde_json::from_value(value)?),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(Error::Closed),
        }
    }

    /// Sends one message with no answer expected.
    ///
    /// # Errors
    /// Returns an error when the stream is closed or the message cannot be written.
    pub fn notify<P: Serialize>(&self, method: &str, params: &P) -> Result<(), Error> {
        let params = serde_json::to_value(params)?;
        let line = wire::to_line(&Notification::new(method, params))?;
        self.out
            .send(Outbound::Line(line))
            .map_err(|_| Error::Closed)
    }

    /// Answers a question from the engine.
    ///
    /// # Errors
    /// Returns an error when the stream is closed or the answer cannot be written.
    pub fn answer<R: Serialize>(&self, id: Id, result: &R) -> Result<(), Error> {
        let result = serde_json::to_value(result)?;
        let line = wire::to_line(&Response::new(id, result))?;
        self.out
            .send(Outbound::Line(line))
            .map_err(|_| Error::Closed)
    }

    /// The next thing the engine sent; `None` when the stream ended.
    pub async fn recv(&mut self) -> Option<Incoming> {
        self.incoming.recv().await
    }

    /// `initialize`: agree on the protocol version.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn initialize(&self, params: &Initialize) -> Result<InitializeResult, Error> {
        self.request(method::INITIALIZE, params).await
    }

    /// `shutdown`: close everything.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn shutdown(&self) -> Result<Done, Error> {
        self.request(method::SHUTDOWN, &Done {}).await
    }

    /// `workspace/open`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn workspace_open(
        &self,
        params: &WorkspaceOpen,
    ) -> Result<WorkspaceOpenResult, Error> {
        self.request(method::WORKSPACE_OPEN, params).await
    }

    /// `status/get`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn status_get(&self, params: &StatusGet) -> Result<StatusGetResult, Error> {
        self.request(method::STATUS_GET, params).await
    }

    /// `connection/close`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn connection_close(&self, params: &ConnectionClose) -> Result<Done, Error> {
        self.request(method::CONNECTION_CLOSE, params).await
    }

    /// `service/control`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn service_control(&self, params: &ServiceControl) -> Result<Done, Error> {
        self.request(method::SERVICE_CONTROL, params).await
    }

    /// `releases/list`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn releases_list(&self, params: &ReleasesList) -> Result<ReleasesListResult, Error> {
        self.request(method::RELEASES_LIST, params).await
    }

    /// `release/contents`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn release_contents(
        &self,
        params: &ReleaseContents,
    ) -> Result<ReleaseContentsResult, Error> {
        self.request(method::RELEASE_CONTENTS, params).await
    }

    /// `release/read_file`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn release_read_file(
        &self,
        params: &ReleaseReadFile,
    ) -> Result<ReleaseReadFileResult, Error> {
        self.request(method::RELEASE_READ_FILE, params).await
    }

    /// `logs/follow`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn logs_follow(&self, params: &LogsFollow) -> Result<LogsFollowResult, Error> {
        self.request(method::LOGS_FOLLOW, params).await
    }

    /// `logs/stop`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn logs_stop(&self, params: &LogsStop) -> Result<Done, Error> {
        self.request(method::LOGS_STOP, params).await
    }

    /// `deploy/start`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn deploy_start(&self, params: &DeployStart) -> Result<OperationStarted, Error> {
        self.request(method::DEPLOY_START, params).await
    }

    /// `rollback/start`.
    ///
    /// # Errors
    /// Returns [`Error`] when the call fails or the answer does not fit.
    pub async fn rollback_start(&self, params: &RollbackStart) -> Result<OperationStarted, Error> {
        self.request(method::ROLLBACK_START, params).await
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Reads lines, answers requests, writes what the client sends.
async fn run<R, W>(
    read: R,
    mut write: W,
    mut out: mpsc::UnboundedReceiver<Outbound>,
    shared: Arc<Shared>,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(read).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(text)) => dispatch(&text, &shared),
                Ok(None) | Err(_) => break,
            },
            message = out.recv() => match message {
                Some(Outbound::Line(line)) => {
                    if write.write_all(line.as_bytes()).await.is_err()
                        || write.write_all(b"\n").await.is_err()
                        || write.flush().await.is_err()
                    {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    shared.close();
}

/// Reads one line from the engine into a request, a question, an event or
/// an answer.
fn dispatch(text: &str, shared: &Shared) {
    match serde_json::from_str::<wire::Incoming>(text) {
        Ok(wire::Incoming::Response(response)) => {
            shared.finish(&response.id, Ok(response.result));
        }
        Ok(wire::Incoming::Error(error)) => {
            shared.finish(&error.id, Err(Error::Rpc(error.error)));
        }
        Ok(wire::Incoming::Request(request)) => {
            let Request {
                id, method, params, ..
            } = request;
            match Ask::from_wire(&method, params.clone()) {
                Ok(Some(ask)) => shared.send(Incoming::Ask { id, ask }),
                Ok(None) | Err(_) => shared.send(Incoming::Other {
                    id: Some(id),
                    method,
                    params,
                }),
            }
        }
        Ok(wire::Incoming::Notification(note)) => {
            let Notification { method, params, .. } = note;
            match Event::from_wire(&method, params.clone()) {
                Ok(Some(event)) => shared.send(Incoming::Event(event)),
                Ok(None) => shared.send(Incoming::Other {
                    id: None,
                    method,
                    params,
                }),
                Err(err) => shared.send(Incoming::Malformed {
                    id: None,
                    error: format!("{method}: {err}"),
                }),
            }
        }
        Err(err) => shared.send(Incoming::Malformed {
            id: None,
            error: err.to_string(),
        }),
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod tests;
