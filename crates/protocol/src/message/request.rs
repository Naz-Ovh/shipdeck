//! What an app asks the engine, and what the engine answers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{
    Action, DeployableInfo, FileInfo, Login, Manifest, Problem, ReleaseInfo, ReleaseSource,
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
    /// The folder whose subfolders each hold a `deploy.yaml`.
    pub root: String,
    pub login: Login,
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
    /// The steps it will go through, by their keys.
    pub steps: Vec<String>,
}
