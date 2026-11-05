//! What the engine reports while it works.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::common::{LogKind, Outcome, StepState, Unit};
use super::method;

/// `logs/line`: one new line of a live log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsLine {
    pub view: u64,
    pub line: String,
}

/// `logs/ended`: the live log stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsEnded {
    pub view: u64,
    /// The exit code, when the log ended on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u32>,
    /// Why it ended, when it was stopped for a reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `op/step`: a step of an operation changed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpStep {
    pub op: u64,
    /// Its place in the list `deploy/start` answered with.
    pub step: u32,
    pub state: StepState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `op/progress`: how far the running step is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpProgress {
    pub op: u64,
    pub step: u32,
    pub done: u64,
    /// `0` when the end is not known yet.
    pub total: u64,
    pub unit: Unit,
    /// What is moving, such as `hashing` or a command.
    pub label: String,
}

/// `op/log`: one line of the operation's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpLog {
    pub op: u64,
    pub kind: LogKind,
    pub text: String,
}

/// `op/release`: the release this operation works on, once known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpRelease {
    pub op: u64,
    pub release: String,
}

/// `op/finished`: the operation ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpFinished {
    pub op: u64,
    pub outcome: Outcome,
}

/// One event from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    LogsLine(LogsLine),
    LogsEnded(LogsEnded),
    Step(OpStep),
    Progress(OpProgress),
    Log(OpLog),
    Release(OpRelease),
    Finished(OpFinished),
}

impl Event {
    /// Reads the event `method` carries, or `Ok(None)` for a method this
    /// version does not know.
    ///
    /// # Errors
    /// Returns an error when the params do not fit the method.
    pub fn from_wire(method: &str, params: Value) -> Result<Option<Self>, serde_json::Error> {
        let event = match method {
            method::LOGS_LINE => Self::LogsLine(serde_json::from_value(params)?),
            method::LOGS_ENDED => Self::LogsEnded(serde_json::from_value(params)?),
            method::OP_STEP => Self::Step(serde_json::from_value(params)?),
            method::OP_PROGRESS => Self::Progress(serde_json::from_value(params)?),
            method::OP_LOG => Self::Log(serde_json::from_value(params)?),
            method::OP_RELEASE => Self::Release(serde_json::from_value(params)?),
            method::OP_FINISHED => Self::Finished(serde_json::from_value(params)?),
            _ => return Ok(None),
        };
        Ok(Some(event))
    }
}
