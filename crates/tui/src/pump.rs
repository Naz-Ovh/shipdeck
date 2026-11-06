//! The pipe from the engine, turned into the three channels the screens
//! already read: operations, questions, and the answers of single requests.
//!
//! The app keeps one view of the world; this is where the engine's messages
//! become the events that view was built on.

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use shipdeck_protocol::connection::{Connection, Handle, Incoming};
use shipdeck_protocol::message::ask::Ask;
use shipdeck_protocol::message::event::{Event, LogsEnded, LogsLine};
use shipdeck_protocol::wire::Id;

use shipdeck_engine::deploy::report::{OpId, OpUpdate, RollbackQuestion, UnchangedQuestion};
use shipdeck_engine::ssh::Question;

use crate::app::Fetched;
use crate::convert;

/// Where the pipe's messages go.
pub struct Inboxes {
    pub ops: UnboundedSender<OpUpdate>,
    pub questions: UnboundedSender<Question>,
    pub fetched: UnboundedSender<Fetched>,
}

/// Reads the engine until the pipe closes. Dropping the app drops the
/// channels, and every send below then quietly stops.
pub fn run(connection: Connection, inboxes: Inboxes) {
    let handle = connection.handle();
    tokio::spawn(async move {
        let mut connection = connection;
        while let Some(incoming) = connection.recv().await {
            match incoming {
                Incoming::Event(Event::LogsLine(LogsLine { view, line })) => {
                    let _ = inboxes.fetched.send(Fetched::LogLine { view, line });
                }
                Incoming::Event(Event::LogsEnded(LogsEnded { view, code, error })) => {
                    let result = error.map_or(Ok(code), Err);
                    let _ = inboxes.fetched.send(Fetched::LogsEnded { view, result });
                }
                Incoming::Event(event) => {
                    if let Some(update) = convert::update(event) {
                        let _ = inboxes.ops.send(update);
                    }
                }
                Incoming::Ask { id, ask } => reply(&handle, &inboxes, id, ask),
                // The engine reports through events and asks; anything else
                // is a message this version does not read.
                Incoming::Other { .. } | Incoming::Malformed { .. } => {}
            }
        }
    });
}

/// One question: a dialog for the screens to show, or an answer the engine
/// waits for.
fn reply(handle: &Handle, inboxes: &Inboxes, id: Id, ask: Ask) {
    match ask {
        Ask::Secret(question) => {
            let (reply, answer) = oneshot::channel();
            let _ = inboxes.questions.send(Question::Secret {
                title: question.title,
                prompt: question.prompt,
                reply,
            });
            let handle = handle.clone();
            tokio::spawn(async move {
                if let Ok(secret) = answer.await {
                    let _ = handle.answer(id, &shipdeck_protocol::message::SecretAnswer { secret });
                }
            });
        }
        Ask::TrustHost(question) => {
            let (reply, answer) = oneshot::channel();
            let _ = inboxes.questions.send(Question::TrustHost {
                host: question.host,
                algorithm: question.algorithm,
                fingerprint: question.fingerprint,
                reply,
            });
            let handle = handle.clone();
            tokio::spawn(async move {
                if let Ok(trusted) = answer.await {
                    let _ = handle.answer(id, &trusted);
                }
            });
        }
        Ask::Rollback(question) => {
            let (reply, answer) = oneshot::channel();
            let op: OpId = question.op;
            let asked = RollbackQuestion {
                release: question.release,
                previous: question.previous,
                step: question.step,
                error: question.error,
                seconds: question.seconds,
                reply,
            };
            let _ = inboxes.ops.send(OpUpdate {
                op,
                event: shipdeck_engine::deploy::report::OpEvent::AskRollback(asked),
            });
            answer_bool(handle, id, answer);
        }
        Ask::Unchanged(question) => {
            let (reply, answer) = oneshot::channel();
            let op: OpId = question.op;
            let asked = UnchangedQuestion {
                release: question.release,
                live: question.live,
                reply,
            };
            let _ = inboxes.ops.send(OpUpdate {
                op,
                event: shipdeck_engine::deploy::report::OpEvent::AskUnchanged(asked),
            });
            answer_bool(handle, id, answer);
        }
    }
}

/// Answers the engine with what the dialog said, unless the dialog went away
/// first; then the engine's own safe choice stands.
fn answer_bool(handle: &Handle, id: Id, answer: oneshot::Receiver<bool>) {
    let handle = handle.clone();
    tokio::spawn(async move {
        if let Ok(value) = answer.await {
            let _ = handle.answer(id, &value);
        }
    });
}

#[cfg(test)]
#[path = "pump_test.rs"]
mod tests;
