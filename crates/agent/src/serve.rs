//! The agent's JSON-RPC loop over an SSH channel.

use std::collections::VecDeque;

use anyhow::{Result, bail};
use shipdeck_protocol::connection::{Connection, Incoming};
use shipdeck_protocol::wire::{ErrorObject, code};
use tokio::io::{AsyncRead, AsyncWrite};

/// Serves requests until shutdown or the end of the stream.
///
/// # Errors
/// Returns an error when an answer cannot be written.
pub async fn serve<R, W>(read: R, write: W) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut connection = Connection::start(read, write);
    let mut locks = crate::locks::ProjectLocks::default();
    let logs = crate::logs::Logs::default();
    let mut pending = VecDeque::new();
    loop {
        let incoming = match pending.pop_front() {
            Some(incoming) => incoming,
            None => match connection.recv().await {
                Some(incoming) => incoming,
                None => break,
            },
        };
        match incoming {
            Incoming::Other {
                id: Some(id),
                method,
                params,
            } => {
                if dispatch(
                    &mut connection,
                    &mut locks,
                    &logs,
                    &mut pending,
                    id,
                    &method,
                    params,
                )
                .await?
                {
                    break;
                }
            }
            Incoming::Ask { id, .. } => connection.answer_error(
                id,
                ErrorObject::new(code::METHOD_NOT_FOUND, "this agent does not take questions"),
            )?,
            Incoming::Malformed {
                id: Some(id),
                error,
            } => {
                connection.answer_error(id, ErrorObject::new(code::PARSE_ERROR, error))?;
            }
            Incoming::Other { id: None, .. }
            | Incoming::Malformed { id: None, .. }
            | Incoming::Event(_) => {}
        }
    }
    connection.flush().await;
    Ok(())
}

/// Cancels a running request if the control input closes.
async fn dispatch(
    connection: &mut Connection,
    locks: &mut crate::locks::ProjectLocks,
    logs: &crate::logs::Logs,
    pending: &mut VecDeque<Incoming>,
    id: shipdeck_protocol::wire::Id,
    method: &str,
    params: serde_json::Value,
) -> Result<bool> {
    let handle = connection.handle();
    let work = crate::dispatch::request(&handle, locks, logs, id, method, params);
    tokio::pin!(work);
    loop {
        tokio::select! {
            biased;
            result = &mut work => return result,
            next = connection.recv() => match next {
                Some(incoming) if pending.len() < 128 => pending.push_back(incoming),
                Some(_) => bail!("too many pending agent requests"),
                None => return Ok(true),
            }
        }
    }
}

#[cfg(test)]
#[path = "serve_test.rs"]
mod tests;
