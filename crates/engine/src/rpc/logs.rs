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
        let deployable = session.deployable(&asked.deployable)?;
        let server = session.server(&deployable).await?;
        Ok::<(Server, u64), ErrorObject>((server, session.next_view()))
    }
    .await;
    let (server, view) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return session.reply_error(id, error),
    };
    // The answer goes first: the app needs the view before its first line.
    session.reply(id, convert::value(&LogsFollowResult { view }));
    stream(session, view, server);
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
fn stream(session: &Arc<Session>, view: u64, server: Server) {
    let connection = session.connection();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    session.hold_view(view, stop);
    let held = Arc::clone(session);
    tokio::spawn(async move {
        let result = server
            .follow_logs(
                |line| {
                    let line = LogsLine {
                        view,
                        line: line.to_string(),
                    };
                    let _ = connection.notify(method::LOGS_LINE, &line);
                },
                async {
                    // The view was stopped, or the app is gone.
                    let _ = stopped.await;
                },
            )
            .await;
        let (code, error) = match &result {
            Ok(code) => (*code, None),
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        let ended = LogsEnded { view, code, error };
        let _ = connection.notify(method::LOGS_ENDED, &ended);
        held.stop_view(view);
    });
}
