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

/// `workspace/open`: pick the folder shipdeck runs in, and how to log in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceOpen {
    /// The folder shipdeck runs in: the project folder.
    pub root: String,
    /// The root file named by `--config`; the folder's own root file is not
    /// looked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// Deploy files named on the command line. Each is read as it is, and no
    /// root file is needed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deploy_files: Vec<String>,
    pub login: Login,
    /// Server commands skip the login shell's startup files, on every server.
    #[serde(default)]
    pub no_shell_env: bool,
    /// The env file named by `--env-file`. Without it, `.env` in `root` is
    /// read when it exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_file: Option<String>,
    /// The server details the flags set, for a run with one deploy.
    #[serde(default, skip_serializing_if = "SshOverride::is_empty")]
    pub ssh: SshOverride,
    /// The root file's deploys this run uses, by name. Empty: every deploy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run: Vec<String>,
}

/// `--host`, `--port` and `--user`, as typed. The engine checks them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SshOverride {
    /// `--host`: replaces `server.ssh.host`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// `--port`: replaces `server.ssh.port`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// `--user`: replaces `server.ssh.user`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl SshOverride {
    /// Whether no flag is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.host.is_none() && self.port.is_none() && self.user.is_none()
    }
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
    /// What a failure after the switch does; overrides `release.steps.rollback.mode`.
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
