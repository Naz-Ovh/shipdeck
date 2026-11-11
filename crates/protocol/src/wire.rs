//! The JSON-RPC 2.0 envelope every message travels in: one message per
//! line, every line UTF-8.

use std::fmt::{self, Display};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// What the `jsonrpc` field always says.
const VERSION_TEXT: &str = "2.0";

/// JSON-RPC's own error codes; shipdeck's start at [`code::UNKNOWN_DEPLOYABLE`].
pub mod code {
    /// The line was not JSON.
    pub const PARSE_ERROR: i32 = -32700;
    /// The JSON was not a request, a notification or an answer.
    pub const INVALID_REQUEST: i32 = -32600;
    /// The method does not exist on this side.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// The params do not fit the method.
    pub const INVALID_PARAMS: i32 = -32602;
    /// The side failed while running the method.
    pub const INTERNAL_ERROR: i32 = -32603;

    /// The project named is not in the workspace.
    pub const UNKNOWN_DEPLOYABLE: i32 = -32001;
    /// Another operation is already running on it.
    pub const BUSY: i32 = -32006;
    /// `workspace/open` has not run yet.
    pub const WORKSPACE_NOT_OPEN: i32 = -32007;
    /// The two sides speak different protocol versions.
    pub const PROTOCOL_VERSION: i32 = -32008;
    /// The deploy is listed, but cannot run: a variable it names is not set.
    pub const DEPLOYABLE_UNAVAILABLE: i32 = -32010;
}

/// The `jsonrpc` field: always `2.0`, and refused when it is not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Version;

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(VERSION_TEXT)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text == VERSION_TEXT {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(format!(
                "jsonrpc must be {VERSION_TEXT:?}, not {text:?}"
            )))
        }
    }
}

/// A JSON-RPC id: the number one side picked, or a text it chose instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(u64),
    Text(String),
}

impl Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(f, "{number}"),
            Self::Text(text) => f.write_str(text),
        }
    }
}

impl From<u64> for Id {
    fn from(number: u64) -> Self {
        Self::Number(number)
    }
}

/// A request: an id, a method, and the params it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: Version,
    pub id: Id,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    #[must_use]
    pub fn new(id: Id, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: Version,
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// A message with no id: an event, which is never answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: Version,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    #[must_use]
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: Version,
            method: method.to_string(),
            params,
        }
    }
}

/// An answer that worked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: Version,
    pub id: Id,
    pub result: Value,
}

impl Response {
    #[must_use]
    pub fn new(id: Id, result: Value) -> Self {
        Self {
            jsonrpc: Version,
            id,
            result,
        }
    }
}

/// An answer that did not work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub jsonrpc: Version,
    pub id: Id,
    pub error: ErrorObject,
}

impl ErrorResponse {
    #[must_use]
    pub fn new(id: Id, error: ErrorObject) -> Self {
        Self {
            jsonrpc: Version,
            id,
            error,
        }
    }
}

/// What went wrong, with one of [`code`]'s numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorObject {
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl Display for ErrorObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for ErrorObject {}

/// One line from the other side, before it is known what it is.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Request(Request),
    Notification(Notification),
    Response(Response),
    Error(ErrorResponse),
}

/// The message as one line of JSON, without its end of line.
///
/// # Errors
/// Returns an error when the message cannot be written as JSON.
pub fn to_line<T: Serialize>(message: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(message)
}

#[cfg(test)]
#[path = "wire_test.rs"]
mod tests;
