//! What an app asks the engine, and what the engine answers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{
    Action, DeployableInfo, FileInfo, Login, Manifest, OnFailure, Problem, ReleaseInfo,
    ReleaseSource,
};

/// The answer to a request that has nothing to report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Done {}

/// `initialize`: agree on the protocol version and what this engine can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Initialize {
    /// The version the app speaks; 1 is the first.
    pub protocol: u32,
    /// Who is talking, such as `shipdeck-tui 0.7.0`.
    pub client: String,
}

/// What `initialize` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InitializeResult {
    pub protocol: u32,
    /// The engine's version.
    pub shipdeck: String,
    /// What this engine can do; new words may be added, never renamed.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// `workspace/open`: pick the folder of projects, and how to log in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceOpen {
    /// The folder whose subfolders each hold a `shipdeck.yaml`.
    pub root: String,
    pub login: Login,
    /// Server commands skip the login shell's startup files, on every server.
    #[serde(default)]
    pub no_shell_env: bool,
}

/// What `workspace/open` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceOpenResult {
    pub deployables: Vec<DeployableInfo>,
    /// Every config mistake, in every file; nothing runs until they are fixed.
    #[serde(default)]
    pub problems: Vec<Problem>,
}

/// `status/get`: one look at a project's service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusGet {
    pub deployable: String,
}

/// `server/info`: facts about the server for one deployable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServerInfoGet {
    pub deployable: String,
}

/// What guided setup may grant on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupAccess {
    /// Password-free sudo for every command.
    Full,
    /// Password-free sudo for exactly the commands shipdeck runs.
    Commands,
}

/// `server/setup`: prepare a server once, from the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServerSetup {
    pub deployable: String,
    pub access: SetupAccess,
    /// The user's sudo password, for this run only. It is never saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sudo_password: Option<String>,
}

/// What guided setup did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupReport {
    pub os: String,
    pub user: String,
    pub dir: String,
    /// `root`, `full sudo` or `commands`.
    pub access: String,
    /// Packages this run installed.
    #[serde(default)]
    pub installed: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// `work/get`: the live release, unfinished files, and recent events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkGet {
    pub deployable: String,
}

/// What is known about interrupted work on one server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkReport {
    pub current: Option<String>,
    /// The service's live state, when it could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// Paths a failed or interrupted operation left, relative to the project.
    pub unfinished: Vec<String>,
    /// A release whose activation started and was never confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertain: Option<String>,
    /// The newest saved events for the project, oldest first.
    #[serde(default)]
    pub events: Vec<crate::message::agent::AgentEvent>,
}

/// `work/cleanup`: remove unfinished files after the user approved them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkCleanup {
    pub deployable: String,
    /// The exact unfinished paths `work/get` reported.
    pub artifacts: Vec<String>,
}

/// What the approved cleanup removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkCleanupResult {
    pub removed: Vec<String>,
}

/// `events/read`: replay saved events after a sequence number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsRead {
    pub deployable: String,
    /// Return events whose sequence is greater than this.
    pub sequence: i64,
    /// At most this many; the engine caps it.
    pub limit: u32,
}

/// One page of saved events, in sequence order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventsReadResult {
    pub events: Vec<crate::message::agent::AgentEvent>,
}

/// What `status/get` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusGetResult {
    /// The service folder exists on the server.
    pub dir_exists: bool,
    /// The release `current` points to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// `active`, `failed` and the words systemd and PM2 share.
    pub state: String,
}

/// `connection/close`: drop this server's connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionClose {
    pub deployable: String,
}

/// `service/control`: start, stop or restart the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceControl {
    pub deployable: String,
    pub action: Action,
}

/// `releases/list`: every release on the server, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleasesList {
    pub deployable: String,
}

/// What `releases/list` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleasesListResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    pub entries: Vec<ReleaseInfo>,
}

/// `release/contents`: what one release holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseContents {
    pub deployable: String,
    pub release: String,
    pub source: ReleaseSource,
}

/// What `release/contents` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseContentsResult {
    pub files: Vec<FileInfo>,
    /// Only a backup can show it without unpacking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Manifest>,
}

/// `release/read_file`: the start of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseReadFile {
    pub deployable: String,
    pub release: String,
    pub source: ReleaseSource,
    /// Relative to the release, `/`-separated.
    pub path: String,
}

/// What `release/read_file` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseReadFileResult {
    /// At most 512 KiB of the file, as text.
    pub text: String,
    /// The file holds a NUL byte, so it is not shown as text.
    pub binary: bool,
}

/// `logs/follow`: start a live log of the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsFollow {
    pub deployable: String,
}

/// What `logs/follow` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsFollowResult {
    /// The id each `logs/line` and `logs/ended` carries.
    pub view: u64,
}

/// `logs/stop`: stop a live log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogsStop {
    pub view: u64,
}

/// `deploy/start`: build and deploy the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeployStart {
    pub deployable: String,
    /// What a failure after the switch does; overrides `pipeline.failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<OnFailure>,
}

/// `rollback/start`: put one release back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RollbackStart {
    pub deployable: String,
    pub release: String,
}

/// What `deploy/start` and `rollback/start` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OperationStarted {
    /// The id every `op/*` event of this work carries.
    pub op: u64,
    /// The steps it will go through, by the names the engine shows.
    pub steps: Vec<String>,
}
