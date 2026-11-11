//! The loop: one message in, one answer out, until the app says `shutdown`
//! or closes the stream.

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::JoinSet;

use shipdeck_protocol::connection::{Connection, Incoming};
use shipdeck_protocol::message::method;
use shipdeck_protocol::message::request::Done;
use shipdeck_protocol::wire::{ErrorObject, Id, code};

use super::session::Session;
use super::{convert, logs, operations, releases, service, work, workspace};

/// Serves one app until it is done. Nothing is written to stdout but the
/// protocol's own lines.
pub async fn run<R, W>(read: R, write: W)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut connection = Connection::start(read, write);
    let session = Arc::new(Session::new(connection.handle()));
    let mut running = JoinSet::new();
    loop {
        tokio::select! {
            incoming = connection.recv() => {
                let Some(incoming) = incoming else { break };
                match incoming {
                    Incoming::Other { id: Some(id), method, params } => {
                        match method.as_str() {
                            method::SHUTDOWN => {
                                session.reply(id, convert::value(&Done {}));
                                break;
                            }
                            // These touch nothing but this process, and the
                            // app's order matters for them: the next message
                            // may name a project or a version it sets here.
                            method::INITIALIZE | method::WORKSPACE_OPEN => {
                                handle(&session, id, method, params).await;
                            }
                            // One task each: a slow call must not hold up the
                            // next one, since its answer may be what it waits
                            // for.
                            _ => {
                                let session = Arc::clone(&session);
                                running.spawn(
                                    async move { handle(&session, id, method, params).await },
                                );
                            }
                        }
                    }
                    // A question is the engine's to ask, not the app's.
                    Incoming::Ask { id, .. } => session.reply_error(
                        id,
                        ErrorObject::new(
                            code::METHOD_NOT_FOUND,
                            "a question is the engine's to ask, not the app's",
                        ),
                    ),
                    // The engine has no notification of its own to read, and a
                    // broken line has no id to answer.
                    Incoming::Other { id: None, .. }
                    | Incoming::Event(_)
                    | Incoming::Malformed { .. } => {}
                }
            }
            // Requests that are done are reaped, so the set does not grow.
            Some(_) = running.join_next() => {}
        }
    }
    // Whatever was asked before the end is still answered. Answers to the
    // engine's own questions come in on the reader, not through this loop.
    while running.join_next().await.is_some() {}
    session.close().await;
    connection.flush().await;
}

/// Sends one request to the method that answers it.
async fn handle(session: &Arc<Session>, id: Id, method: String, params: Value) {
    match method.as_str() {
        method::INITIALIZE => workspace::initialize(session, id, params).await,
        method::WORKSPACE_OPEN => workspace::open(session, id, params).await,
        method::STATUS_GET => service::status(session, id, params).await,
        method::SERVER_INFO => service::server_info(session, id, params).await,
        method::WORK_GET => work::get(session, id, params).await,
        method::WORK_CLEANUP => work::cleanup(session, id, params).await,
        method::EVENTS_READ => work::events(session, id, params).await,
        method::CONNECTION_CLOSE => service::close(session, id, params).await,
        method::SERVICE_CONTROL => service::control(session, id, params).await,
        method::RELEASES_LIST => releases::list(session, id, params).await,
        method::RELEASE_CONTENTS => releases::contents(session, id, params).await,
        method::RELEASE_READ_FILE => releases::read_file(session, id, params).await,
        method::LOGS_FOLLOW => logs::follow(session, id, params).await,
        method::LOGS_STOP => logs::stop(session, id, params).await,
        method::DEPLOY_START => operations::deploy(session, id, params).await,
        method::ROLLBACK_START => operations::rollback(session, id, params).await,
        _ => session.reply_error(
            id,
            ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("this engine has no method {method:?}"),
            ),
        ),
    }
}

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;
