//! The messages of version 1: requests, events and the engine's questions.

pub mod agent;
pub mod ask;
pub mod common;
pub mod event;
pub mod method;
pub mod request;

pub use agent::{
    AgentBackupCreate, AgentBackupCreated, AgentBackupRestore, AgentCleaned, AgentCleanupWork,
    AgentDiscard, AgentDropRequest, AgentEvent, AgentEventAppend, AgentEventSaved, AgentEvents,
    AgentEventsAfter, AgentEventsRecent, AgentFileInfo, AgentFileKind, AgentHello,
    AgentHelloResult, AgentLinkShared, AgentLogLine, AgentLogsEnded, AgentLogsFollow,
    AgentLogsStop, AgentLogsView, AgentManager, AgentOk, AgentPing, AgentRelease,
    AgentReleaseContents, AgentReleaseContentsRequest, AgentReleaseFile, AgentReleaseFiles,
    AgentReleaseFilesRequest, AgentReleaseKind, AgentReleaseRead, AgentReleaseReadRequest,
    AgentReleases, AgentReleasesRequest, AgentService, AgentServiceAction, AgentServiceControl,
    AgentShutdown, AgentStatus, HashCurrent, HashCurrentResult, InspectWork, ProjectLock,
    ServerFacts, ServerFactsRequest, SwitchRelease, UnpackRelease, UnpackResult, UploadPrepare,
    UploadReservation, WorkInspection,
};
pub use ask::{Ask, AskRollback, AskSecret, AskTrustHost, AskUnchanged, SecretAnswer};
pub use common::{
    Action, DeployableInfo, FileInfo, FileKind, LogKind, Login, Manifest, OnFailure, Outcome,
    Problem, ReleaseInfo, ReleaseKind, ReleaseName, ReleaseSource, StepState, Unit,
};
pub use event::{Event, LogsEnded, LogsLine, OpFinished, OpLog, OpProgress, OpRelease, OpStep};
pub use request::{
    ConnectionClose, DeployStart, Done, EventsRead, EventsReadResult, Initialize, InitializeResult,
    LogsFollow, LogsFollowResult, LogsStop, OperationStarted, ReleaseContents,
    ReleaseContentsResult, ReleaseReadFile, ReleaseReadFileResult, ReleasesList,
    ReleasesListResult, RollbackStart, ServerInfoGet, ServerSetup, ServiceControl, SetupAccess,
    SetupReport, SshOverride, StatusGet, StatusGetResult, WorkCleanup, WorkCleanupResult, WorkGet,
    WorkReport, WorkspaceOpen, WorkspaceOpenResult,
};

#[cfg(test)]
#[path = "message_test.rs"]
mod tests;
