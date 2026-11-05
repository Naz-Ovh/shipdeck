//! The questions the engine asks, and what their answers carry.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::method;

/// `ask/rollback`: the new release failed. Roll back?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskRollback {
    pub op: u64,
    /// The release that failed.
    pub release: String,
    /// The release that was live before it.
    pub previous: String,
    /// The step that failed, such as `health`.
    pub step: String,
    pub error: String,
    /// No answer in this many seconds means yes.
    pub seconds: u64,
}

/// `ask/unchanged`: the release holds what the live one holds. Deploy it?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskUnchanged {
    pub op: u64,
    /// The release the deploy would make.
    pub release: String,
    /// The live release with the same content.
    pub live: String,
}

/// `ask/secret`: a password, a passphrase, or another secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskSecret {
    /// A short name for the dialog, such as `SSH password`.
    pub title: String,
    /// The question itself, with the server it is for.
    pub prompt: String,
}

/// `ask/trust_host`: a server sent a host key this PC has not seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskTrustHost {
    pub host: String,
    /// The key's algorithm, such as `ssh-ed25519`.
    pub algorithm: String,
    /// As `ssh-keygen -l` prints it, such as `SHA256:...`.
    pub fingerprint: String,
}

/// The answer to `ask/secret`; `None` means the question was cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// A question the engine asked, with the id its answer must carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Rollback(AskRollback),
    Unchanged(AskUnchanged),
    Secret(AskSecret),
    TrustHost(AskTrustHost),
}

impl Ask {
    /// Reads the question `method` carries, or `Ok(None)` for a method this
    /// version does not know.
    ///
    /// # Errors
    /// Returns an error when the params do not fit the method.
    pub fn from_wire(method: &str, params: Value) -> Result<Option<Self>, serde_json::Error> {
        let ask = match method {
            method::ASK_ROLLBACK => Self::Rollback(serde_json::from_value(params)?),
            method::ASK_UNCHANGED => Self::Unchanged(serde_json::from_value(params)?),
            method::ASK_SECRET => Self::Secret(serde_json::from_value(params)?),
            method::ASK_TRUST_HOST => Self::TrustHost(serde_json::from_value(params)?),
            _ => return Ok(None),
        };
        Ok(Some(ask))
    }
}
