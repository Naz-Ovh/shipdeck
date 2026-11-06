//! `logs/follow` and `logs/stop`: a live log of the service, until it ends.

use std::sync::Arc;

use serde_json::Value;

use shipdeck_protocol::message::event::{LogsEnded, LogsLine};
use shipdeck_protocol::message::method;
use shipdeck_protocol::message::request::{Done, LogsFollow, LogsFollowResult, LogsStop};
use shipdeck_protocol::wire::{ErrorObject, Id};

use crate::remote::Server;

use super::convert;
use super::session::Session;

/// Starts following the service's log. The answer carries the view every line
/// of this log is tagged with.
pub async fn follow(session: &Arc<Session>, id: Id, params: Value) {
    let prepared = async {
        let asked: LogsFollow = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        Ok::<(Vec<Server>, u64), ErrorObject>((servers, session.next_view()))
    }
    .await;
    let (servers, view) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return session.reply_error(id, error),
    };
    // The answer goes first: the app needs the view before its first line.
    session.reply(id, convert::value(&LogsFollowResult { view }));
    stream(session, view, servers);
}

/// Stops a live log; its view still gets `logs/ended`.
pub async fn stop(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: LogsStop = convert::params(params)?;
        session.stop_view(asked.view);
        convert::value(&Done {})
    }
    .await;
    session.reply(id, reply);
}

/// Follows the log until it ends on its own or is stopped, then says how it
/// ended and forgets the view.
fn stream(session: &Arc<Session>, view: u64, servers: Vec<Server>) {
    let connection = session.connection();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    session.hold_view(view, stop);
    let held = Arc::clone(session);
    tokio::spawn(async move {
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let mut tasks = tokio::task::JoinSet::new();
        let multiple = servers.len() > 1;
        for server in servers {
            let connection = connection.clone();
            let mut cancelled = cancelled.clone();
            tasks.spawn(async move {
                let label = server.process_name().to_string();
                server
                    .follow_logs(
                        |line| {
                            let line = LogsLine {
                                view,
                                line: if multiple {
                                    format!("[{label}] {line}")
                                } else {
                                    line.to_string()
                                },
                            };
                            let _ = connection.notify(method::LOGS_LINE, &line);
                        },
                        async {
                            let _ = cancelled.changed().await;
                        },
                    )
                    .await
            });
        }
        let mut stopped = std::pin::pin!(stopped);
        let mut error = None;
        let mut code = None;
        while !tasks.is_empty() {
            tokio::select! {
                _ = &mut stopped => { let _ = cancel.send(true); break; }
                result = tasks.join_next() => {
                    match result {
                        Some(Ok(Ok(exit))) => code = exit,
                        Some(Ok(Err(err))) => error = Some(format!("{err:#}")),
                        Some(Err(err)) => error = Some(err.to_string()),
                        None => break,
                    }
                }
            }
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok(Err(err)) = result {
                error = Some(format!("{err:#}"));
            }
        }
        let ended = LogsEnded { view, code, error };
        let _ = connection.notify(method::LOGS_ENDED, &ended);
        held.stop_view(view);
    });
}
