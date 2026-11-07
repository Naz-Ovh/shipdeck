//! The messages of version 1: requests, events and the engine's questions.

pub mod agent;
pub mod ask;
pub mod common;
pub mod event;
pub mod method;
pub mod request;

pub use agent::{AgentHello, AgentHelloResult, AgentPing, AgentShutdown};
pub use ask::{Ask, AskRollback, AskSecret, AskTrustHost, AskUnchanged, SecretAnswer};
pub use common::{
    Action, DeployableInfo, FileInfo, FileKind, LogKind, Login, Manifest, OnFailure, Outcome,
    Problem, ReleaseInfo, ReleaseKind, ReleaseName, ReleaseSource, StepState, Unit,
};
pub use event::{Event, LogsEnded, LogsLine, OpFinished, OpLog, OpProgress, OpRelease, OpStep};
pub use request::{
    ConnectionClose, DeployStart, Done, Initialize, InitializeResult, LogsFollow, LogsFollowResult,
    LogsStop, OperationStarted, ReleaseContents, ReleaseContentsResult, ReleaseReadFile,
    ReleaseReadFileResult, ReleasesList, ReleasesListResult, RollbackStart, ServiceControl,
    StatusGet, StatusGetResult, WorkspaceOpen, WorkspaceOpenResult,
};

#[cfg(test)]
#[path = "message_test.rs"]
mod tests;
