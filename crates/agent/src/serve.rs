//! The agent's loop: answer typed requests until `agent/shutdown` or the end
//! of the stream. Unsupported requests that carry an id get an error back,
//! never silence.

use anyhow::Result;
use shipdeck_protocol::connection::{Connection, Incoming};
use shipdeck_protocol::message::agent::{
    AgentHello, AgentHelloResult, AgentPing, AgentShutdown, HELLO, PING, PROTOCOL, SHUTDOWN,
};
use shipdeck_protocol::wire::{ErrorObject, code};
use tokio::io::{AsyncRead, AsyncWrite};

/// Serves requests until `agent/shutdown` or the end of the stream.
///
/// # Errors
///
/// Returns an error when an answer cannot be written.
pub async fn serve<R, W>(read: R, write: W) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut connection = Connection::start(read, write);
    while let Some(incoming) = connection.recv().await {
        match incoming {
            Incoming::Other {
                id: Some(id),
                method,
                params,
            } => match method.as_str() {
                HELLO => match serde_json::from_value::<AgentHello>(params) {
                    Ok(asked) if asked.protocol == PROTOCOL => {
                        connection.answer(
                            id,
                            &AgentHelloResult {
                                protocol: PROTOCOL,
                                version: env!("CARGO_PKG_VERSION").to_string(),
                                os: std::env::consts::OS.to_string(),
                                arch: std::env::consts::ARCH.to_string(),
                            },
                        )?;
                    }
                    Ok(asked) => {
                        connection.answer_error(
                            id,
                            ErrorObject::new(
                                code::PROTOCOL_VERSION,
                                format!(
                                    "this agent speaks protocol {PROTOCOL}, and the engine speaks {}",
                                    asked.protocol
                                ),
                            ),
                        )?;
                        break;
                    }
                    Err(err) => {
                        connection.answer_error(
                            id,
                            ErrorObject::new(code::INVALID_PARAMS, err.to_string()),
                        )?;
                    }
                },
                PING => connection.answer(id, &AgentPing {})?,
                SHUTDOWN => {
                    connection.answer(id, &AgentShutdown {})?;
                    break;
                }
                _ => connection.answer_error(
                    id,
                    ErrorObject::new(
                        code::METHOD_NOT_FOUND,
                        format!("this agent has no method {method:?}"),
                    ),
                )?,
            },
            // A question is the engine's to ask, not the agent's. It still
            // carries an id, so leaving it unanswered would block that caller.
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

#[cfg(test)]
#[path = "serve_test.rs"]
mod tests;
