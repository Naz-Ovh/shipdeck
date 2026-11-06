//! `initialize` and `workspace/open`: which version, and which folder.

use serde_json::Value;

use shipdeck_protocol::VERSION;
use shipdeck_protocol::message::request::{Initialize, InitializeResult, WorkspaceOpen};
use shipdeck_protocol::wire::{ErrorObject, Id, code};

use super::convert;
use super::session::Session;

/// Checks the version the app speaks, and says what this engine is.
///
/// Nothing is optional in version 1, so `capabilities` is empty; a word is
/// added there when an engine can do more than the version requires.
pub async fn initialize(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: Initialize = convert::params(params)?;
        if asked.protocol != VERSION {
            return Err(ErrorObject::new(
                code::PROTOCOL_VERSION,
                format!(
                    "this engine speaks protocol {VERSION}, and the app speaks {}",
                    asked.protocol
                ),
            ));
        }
        convert::value(&InitializeResult {
            protocol: VERSION,
            shipdeck: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: Vec::new(),
        })
    }
    .await;
    session.reply(id, reply);
}

/// Opens a folder of projects; every later message names one of them.
pub async fn open(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: WorkspaceOpen = convert::params(params)?;
        let answer = session.open(&asked).await?;
        convert::value(&answer)
    }
    .await;
    session.reply(id, reply);
}
