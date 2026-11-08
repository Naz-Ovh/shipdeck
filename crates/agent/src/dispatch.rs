//! Maps typed agent requests to their operations.

use std::future::Future;

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use shipdeck_protocol::connection::Handle;
use shipdeck_protocol::message::agent::{
    AgentHello, AgentHelloResult, AgentPing, AgentShutdown, BACKUP_CREATE, BACKUP_RESTORE,
    CLEANUP_WORK, DISCARD_RELEASE, EVENT_APPEND, EVENTS_AFTER, EVENTS_RECENT, HASH_CURRENT, HELLO,
    HashCurrent, INSPECT_WORK, LINK_SHARED, LOCK_PROJECT, LOGS_FOLLOW, LOGS_STOP, PING,
    PREPARE_PROJECT, PROTOCOL, ProjectLock, RELEASE_CONTENTS, RELEASE_DROP, RELEASE_FILES,
    RELEASE_READ_FILE, RELEASES_LIST, SERVER_FACTS, SERVICE_CONTROL, SERVICE_STATUS, SHUTDOWN,
    SWITCH_RELEASE, UNLOCK_PROJECT, UNPACK_RELEASE, UPLOAD_PREPARE,
};
use shipdeck_protocol::wire::{ErrorObject, Id, code};

/// Answers one request; returns true when the connection should close.
pub(crate) async fn request(
    connection: &Handle,
    locks: &mut crate::locks::ProjectLocks,
    logs: &crate::logs::Logs,
    id: Id,
    method: &str,
    params: Value,
) -> Result<bool> {
    if !mutation_allowed(connection, locks, &id, method, &params)? {
        return Ok(false);
    }
    match method {
        HELLO => return hello(connection, id, params),
        PING => connection.answer(id, &AgentPing {})?,
        SHUTDOWN => {
            connection.answer(id, &AgentShutdown {})?;
            return Ok(true);
        }
        EVENT_APPEND => operation(connection, id, params, crate::state::append).await?,
        EVENTS_AFTER => operation(connection, id, params, crate::state::after).await?,
        EVENTS_RECENT => operation(connection, id, params, crate::state::recent).await?,
        INSPECT_WORK => operation(connection, id, params, crate::files::inspect_work).await?,
        LINK_SHARED => operation(connection, id, params, crate::files::link_shared).await?,
        RELEASES_LIST => {
            operation(
                connection,
                id,
                params,
                |asked: shipdeck_protocol::message::agent::AgentReleasesRequest| async move {
                    crate::releases::list(asked.root).await
                },
            )
            .await?;
        }
        RELEASE_FILES => operation(connection, id, params, crate::releases::files).await?,
        RELEASE_CONTENTS => {
            operation(connection, id, params, crate::inspect_release::contents).await?;
        }
        RELEASE_READ_FILE => {
            operation(connection, id, params, crate::inspect_release::read).await?;
        }
        RELEASE_DROP => {
            operation(connection, id, params, crate::inspect_release::drop_one).await?;
        }
        BACKUP_CREATE => operation(connection, id, params, crate::backup::create).await?,
        BACKUP_RESTORE => operation(connection, id, params, crate::backup::restore).await?,
        SERVICE_STATUS => operation(connection, id, params, crate::service::status).await?,
        SERVICE_CONTROL => operation(connection, id, params, crate::service::control).await?,
        CLEANUP_WORK => operation(connection, id, params, crate::cleanup::cleanup).await?,
        DISCARD_RELEASE => operation(connection, id, params, crate::cleanup::discard).await?,
        LOGS_FOLLOW => match serde_json::from_value(params) {
            Ok(asked) => match logs.follow(connection, &asked) {
                Ok(result) => connection.answer(id, &result)?,
                Err(err) => connection.answer_error(
                    id,
                    ErrorObject::new(code::INTERNAL_ERROR, format!("{err:#}")),
                )?,
            },
            Err(err) => {
                connection
                    .answer_error(id, ErrorObject::new(code::INVALID_PARAMS, err.to_string()))?;
            }
        },
        LOGS_STOP => {
            operation(
                connection,
                id,
                params,
                |asked: shipdeck_protocol::message::agent::AgentLogsStop| async move {
                    logs.stop(asked);
                    Ok(shipdeck_protocol::message::agent::AgentOk {})
                },
            )
            .await?;
        }
        SERVER_FACTS => operation(connection, id, params, crate::facts::read).await?,
        UNPACK_RELEASE => operation(connection, id, params, crate::unpack::unpack).await?,
        LOCK_PROJECT => lock_operation(connection, locks, id, params, true)?,
        UNLOCK_PROJECT => lock_operation(connection, locks, id, params, false)?,
        PREPARE_PROJECT => operation(connection, id, params, crate::project::prepare).await?,
        SWITCH_RELEASE => operation(connection, id, params, crate::project::switch).await?,
        HASH_CURRENT => {
            operation(connection, id, params, |asked: HashCurrent| async move {
                crate::files::hash_current(&asked).await
            })
            .await?;
        }
        UPLOAD_PREPARE => upload(connection, id, params, locks).await?,
        _ => connection.answer_error(
            id,
            ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("this agent has no method {method:?}"),
            ),
        )?,
    }
    Ok(false)
}

/// Checks the upload's token against the connection's lock, then reserves it.
async fn upload(
    connection: &Handle,
    id: Id,
    params: Value,
    locks: &crate::locks::ProjectLocks,
) -> Result<()> {
    if let Ok(asked) =
        serde_json::from_value::<shipdeck_protocol::message::agent::UploadPrepare>(params.clone())
        && locks.token(&asked.root) != Some(asked.lock_token)
    {
        connection.answer_error(
            id,
            ErrorObject::new(
                code::BUSY,
                "the upload does not belong to this project lock",
            ),
        )?;
        return Ok(());
    }
    operation(connection, id, params, crate::transfer::prepare_recorded).await?;
    Ok(())
}

/// Answers `false` when a change was refused because the lock is not held.
fn mutation_allowed(
    connection: &Handle,
    locks: &crate::locks::ProjectLocks,
    id: &Id,
    method: &str,
    params: &Value,
) -> Result<bool> {
    let Some(root) = mutation_root(method, params) else {
        return Ok(true);
    };
    if locks.holds(root) {
        return Ok(true);
    }
    connection.answer_error(
        id.clone(),
        ErrorObject::new(
            code::BUSY,
            "the project lock is not held; lock the project before changing it",
        ),
    )?;
    Ok(false)
}

/// The root a mutating request names; read-only methods return `None`.
fn mutation_root<'a>(method: &str, params: &'a Value) -> Option<&'a str> {
    match method {
        SERVICE_CONTROL => params
            .get("service")
            .and_then(|service| service.get("root"))
            .and_then(Value::as_str),
        PREPARE_PROJECT | UNPACK_RELEASE | SWITCH_RELEASE | LINK_SHARED | BACKUP_CREATE
        | BACKUP_RESTORE | CLEANUP_WORK | DISCARD_RELEASE | RELEASE_DROP | UPLOAD_PREPARE => {
            params.get("root").and_then(Value::as_str)
        }
        _ => None,
    }
}

fn lock_operation(
    connection: &Handle,
    locks: &mut crate::locks::ProjectLocks,
    id: Id,
    params: Value,
    acquire: bool,
) -> Result<()> {
    match serde_json::from_value::<ProjectLock>(params) {
        Ok(request) => {
            let result = if acquire {
                locks.acquire(&request)
            } else {
                locks.release(&request)
            };
            match result {
                Ok(answer) => connection.answer(id, &answer)?,
                Err(err) => {
                    connection
                        .answer_error(id, ErrorObject::new(code::BUSY, format!("{err:#}")))?;
                }
            }
        }
        Err(err) => {
            connection.answer_error(id, ErrorObject::new(code::INVALID_PARAMS, err.to_string()))?;
        }
    }
    Ok(())
}

fn hello(connection: &Handle, id: Id, params: Value) -> Result<bool> {
    match serde_json::from_value::<AgentHello>(params) {
        Ok(asked) if asked.protocol == PROTOCOL => connection.answer(
            id,
            &AgentHelloResult {
                protocol: PROTOCOL,
                version: env!("CARGO_PKG_VERSION").to_string(),
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
            },
        )?,
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
            return Ok(true);
        }
        Err(err) => {
            connection.answer_error(id, ErrorObject::new(code::INVALID_PARAMS, err.to_string()))?;
        }
    }
    Ok(false)
}

async fn operation<P, R, F, Fut>(connection: &Handle, id: Id, params: Value, work: F) -> Result<()>
where
    P: DeserializeOwned,
    R: Serialize,
    F: FnOnce(P) -> Fut,
    Fut: Future<Output = Result<R>>,
{
    match serde_json::from_value(params) {
        Ok(asked) => match work(asked).await {
            Ok(result) => connection.answer(id, &result)?,
            Err(err) => connection.answer_error(
                id,
                ErrorObject::new(code::INTERNAL_ERROR, format!("{err:#}")),
            )?,
        },
        Err(err) => {
            connection.answer_error(id, ErrorObject::new(code::INVALID_PARAMS, err.to_string()))?;
        }
    }
    Ok(())
}
