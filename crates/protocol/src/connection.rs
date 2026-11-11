//! One end of the pipe: requests out, events and questions in.
//!
//! Both sides use the same type. An app uses [`Connection`] to call the
//! engine; the engine's own loop holds the connection and a [`Handle`] to
//! answer, report and ask from the tasks it starts.

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
    Done, Initialize, InitializeResult, WorkspaceOpen, WorkspaceOpenResult,
};
use crate::wire::{self, ErrorObject, Id, Notification, Request, Response};

/// Why a call to the other side did not finish.
#[derive(Debug)]
pub enum Error {
    /// The stream could not be read or written.
    Io(std::io::Error),
    /// A message was not valid JSON, or not the shape it should be.
    Json(serde_json::Error),
    /// The other side answered with an error.
    Rpc(ErrorObject),
    /// The stream ended; nothing more will arrive.
    Closed,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "the stream failed: {err}"),
            Self::Json(err) => write!(f, "a message was not what it should be: {err}"),
            Self::Rpc(err) => write!(f, "the other side said no: {err}"),
            Self::Closed => f.write_str("the other side closed the stream"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// The message for a person: what the other side said, without the
    /// protocol's own words around it.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Rpc(object) => object.message.clone(),
            other => other.to_string(),
        }
    }
}

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

/// What arrived, other than an answer to a request.
#[derive(Debug)]
pub enum Incoming {
    /// Something the other side reports; nothing to answer.
    Event(Event),
    /// A question, with the id its answer must carry.
    Ask { id: Id, ask: Ask },
    /// A method this version does not know, or whose params do not fit.
    /// An id means the other side waits for an answer.
    Other {
        id: Option<Id>,
        method: String,
        params: Value,
    },
    /// A line that was not a JSON-RPC message this version reads.
    Malformed { id: Option<Id>, error: String },
}

/// The chat with one other side, over a stream that carries one message per
/// line.
///
/// Dropping it closes the stream; the other side then takes no new work.
pub struct Connection {
    handle: Handle,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    task: JoinHandle<()>,
}

/// The sending half of a [`Connection`]: requests, answers and reports.
///
/// Cheap to clone, so the tasks a worker starts can report while its own
/// loop keeps reading.
#[derive(Clone)]
pub struct Handle {
    out: mpsc::UnboundedSender<Outbound>,
    shared: Arc<Shared>,
    next: Arc<AtomicU64>,
}

/// One line this side has to write, or the end of what it writes.
enum Outbound {
    Line(String),
    /// Everything queued before this is on the stream; tell the sender, and
    /// keep the connection open.
    Flushed(oneshot::Sender<()>),
}

/// What the reading task and the handle share.
struct Shared {
    /// Requests waiting for their answer, by id.
    pending: Mutex<HashMap<Id, oneshot::Sender<Result<Value, Error>>>>,
    /// The way what arrives reaches the connection. Dropping it ends `recv`,
    /// so the reading task takes it out when the stream ends.
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
            // The other side may be gone; then nothing is left to tell.
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
        // No new message can arrive; `recv` ends once the buffered ones are read.
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

impl Connection {
    /// Starts a connection on the two halves of one stream: a child
    /// process's pipes, a socket, or a test pipe.
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
            handle: Handle {
                out,
                shared,
                next: Arc::new(AtomicU64::new(1)),
            },
            incoming,
            task,
        }
    }

    /// The sending half, for the tasks a worker starts.
    #[must_use]
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Sends one request and waits for its answer.
    ///
    /// # Errors
    /// Returns an error when the stream closes, the other side answers with
    /// an error, or the answer is not the expected shape.
    pub async fn request<P, R>(&self, method: &str, params: &P) -> Result<R, Error>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        self.handle.request(method, params).await
    }

    /// Sends one message with no answer expected.
    ///
    /// # Errors
    /// Returns an error when the stream is closed or the message cannot be written.
    pub fn notify<P: Serialize>(&self, method: &str, params: &P) -> Result<(), Error> {
        self.handle.notify(method, params)
    }

    /// Answers a request.
    ///
    /// # Errors
    /// Returns an error when the stream is closed or the answer cannot be written.
    pub fn answer<R: Serialize>(&self, id: Id, result: &R) -> Result<(), Error> {
        self.handle.answer(id, result)
    }

    /// Answers a request that could not be carried out.
    ///
    /// # Errors
    /// Returns an error when the stream is closed or the answer cannot be written.
    pub fn answer_error(&self, id: Id, error: ErrorObject) -> Result<(), Error> {
        self.handle.answer_error(id, error)
    }

    /// The next thing the other side sent; `None` when the stream ended.
    pub async fn recv(&mut self) -> Option<Incoming> {
        self.incoming.recv().await
    }

    /// Waits until everything queued is on the stream.
    ///
    /// A worker that answers and then returns must call this: the lines it
    /// queued would otherwise be lost when the connection closes.
    pub async fn flush(&self) {
        let (flushed, done) = oneshot::channel();
        if self.handle.out.send(Outbound::Flushed(flushed)).is_ok() {
            let _ = done.await;
        }
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
}

impl Handle {
    /// Sends one request and waits for its answer.
    ///
    /// # Errors
    /// Returns an error when the stream closes, the other side answers with
    /// an error, or the answer is not the expected shape.
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

    /// Answers a request.
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

    /// Answers a request that could not be carried out.
    ///
    /// # Errors
    /// Returns an error when the stream is closed or the answer cannot be written.
    pub fn answer_error(&self, id: Id, error: ErrorObject) -> Result<(), Error> {
        let line = wire::to_line(&wire::ErrorResponse::new(id, error))?;
        self.out
            .send(Outbound::Line(line))
            .map_err(|_| Error::Closed)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Reads lines, answers requests, writes what this side sends.
///
/// Reading and writing end apart: the answers already queued still go out
/// once nothing more can arrive.
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
    let mut reading = true;
    loop {
        tokio::select! {
            line = lines.next_line(), if reading => {
                if let Ok(Some(text)) = line {
                    dispatch(&text, &shared);
                } else {
                    // Nothing more can arrive; `recv` ends, and the requests
                    // still waiting fail.
                    reading = false;
                    shared.close();
                }
            }
            message = out.recv() => match message {
                Some(Outbound::Line(line)) => {
                    if write.write_all(line.as_bytes()).await.is_err()
                        || write.write_all(b"\n").await.is_err()
                        || write.flush().await.is_err()
                    {
                        break;
                    }
                }
                Some(Outbound::Flushed(flushed)) => {
                    let _ = flushed.send(());
                }
                None => break,
            },
        }
    }
    // A stream that broke ends the other side's reading with it.
    shared.close();
}

/// Reads one line from the other side into a request, a question, an event or
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
#[path = "connection_test.rs"]
mod tests;
