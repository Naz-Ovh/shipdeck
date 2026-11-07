//! `shipdeck-agent`: the small program the engine starts on a server over
//! SSH. It speaks the same JSON-RPC as the app protocol, on stdin and stdout,
//! and exits when the connection ends. It writes nothing else to stdout.

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
        let Incoming::Other {
            id: Some(id),
            method,
            params,
        } = incoming
        else {
            continue;
        };
        match method.as_str() {
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
        }
    }
    connection.flush().await;
    Ok(())
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod tests;
