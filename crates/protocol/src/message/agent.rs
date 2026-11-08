//! The words between the engine and `shipdeck-agent`, the helper the engine
//! starts on a server over SSH. Same envelope as the app protocol; these
//! methods carry an `agent/` prefix so the two sets never meet.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The agent protocol version this crate speaks.
pub const PROTOCOL: u32 = 3;

/// `agent/hello`: agree on the version before anything else.
pub const HELLO: &str = "agent/hello";
/// `agent/ping`: prove the agent is still there.
pub const PING: &str = "agent/ping";
/// `agent/shutdown`: the connection is ending; answer, then exit.
pub const SHUTDOWN: &str = "agent/shutdown";
/// Hash regular files in the current release.
pub const HASH_CURRENT: &str = "agent/hash_current";
/// Reserve one upload on a separate SSH channel.
pub const UPLOAD_PREPARE: &str = "agent/upload_prepare";
/// Save or replay numbered agent events.
pub const EVENT_APPEND: &str = "agent/event_append";
pub const EVENTS_AFTER: &str = "agent/events_after";
/// Inspect the live release and unfinished work.
pub const INSPECT_WORK: &str = "agent/inspect_work";
/// Prepare project directories or switch the live release.
pub const PREPARE_PROJECT: &str = "agent/prepare_project";
pub const SWITCH_RELEASE: &str = "agent/switch_release";
/// Hold or release a project's deployment lock for this SSH session.
pub const LOCK_PROJECT: &str = "agent/lock_project";
pub const UNLOCK_PROJECT: &str = "agent/unlock_project";
/// Safely unpack and verify one prepared release.
pub const UNPACK_RELEASE: &str = "agent/unpack_release";
/// Read server information using the connected agent.
pub const SERVER_FACTS: &str = "agent/server_facts";
/// Read the service's state or change it.
pub const SERVICE_STATUS: &str = "agent/service_status";
pub const SERVICE_CONTROL: &str = "agent/service_control";
/// The releases and backups in the deployment folder.
pub const RELEASES_LIST: &str = "agent/releases_list";
/// Link shared folders into a prepared release.
pub const LINK_SHARED: &str = "agent/link_shared";
/// List a release's files, or create and restore its backup.
pub const RELEASE_FILES: &str = "agent/release_files";
pub const BACKUP_CREATE: &str = "agent/backup_create";
pub const BACKUP_RESTORE: &str = "agent/backup_restore";
/// Follow and stop a service's live log.
pub const LOGS_FOLLOW: &str = "agent/logs_follow";
pub const LOGS_STOP: &str = "agent/logs_stop";
pub const LOGS_LINE: &str = "agent/logs_line";
pub const LOGS_ENDED: &str = "agent/logs_ended";
/// Remove unfinished files after the owner approves, or a failed release.
pub const CLEANUP_WORK: &str = "agent/cleanup_work";
pub const DISCARD_RELEASE: &str = "agent/discard_release";
/// Read recent events for one project.
pub const EVENTS_RECENT: &str = "agent/events_recent";
/// Look inside a release or its backup, and remove one kept release.
pub const RELEASE_CONTENTS: &str = "agent/release_contents";
pub const RELEASE_READ_FILE: &str = "agent/release_read_file";
pub const RELEASE_DROP: &str = "agent/release_drop";

/// The absolute path whose filesystem should be measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServerFactsRequest {
    pub disk_path: String,
}

/// Current Linux system information; byte counts are in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServerFacts {
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_count: u32,
    pub memory_total: u64,
    pub memory_available: u64,
    pub disk_total: u64,
    pub disk_available: u64,
    /// The existing path whose filesystem was measured.
    pub disk_path: String,
    pub uptime_seconds: u64,
    pub tools: BTreeMap<String, Option<String>>,
    pub notes: Vec<String>,
    pub agent_version: String,
}

/// The uploaded bundle and its reuse lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnpackRelease {
    pub root: String,
    pub release: String,
    pub modes: Vec<u32>,
    pub max_bytes: u64,
    pub max_entries: u64,
}

/// The result of checking all files in a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnpackResult {
    pub verified: bool,
    pub mismatches: Vec<String>,
}

/// A project lock and the caller's unique token for this session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectLock {
    pub root: String,
    pub token: u64,
}

/// The root and release name for a live switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SwitchRelease {
    pub root: String,
    pub release: String,
}

/// An operation finished successfully.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentOk {}

/// The deployment directory to inspect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InspectWork {
    pub root: String,
}

/// The current symlink and unfinished release artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkInspection {
    pub current: Option<String>,
    pub unfinished: Vec<String>,
    /// A release whose switch started and whose completion was never saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertain: Option<String>,
}

/// What keeps a service running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentManager {
    Systemd,
    Pm2,
}

/// One service on one deployment, as the agent runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentService {
    /// Absolute deployment directory containing `current`.
    pub root: String,
    /// The systemd unit or PM2 app.
    pub service: String,
    pub manager: AgentManager,
    /// The PM2 ecosystem file, relative to the release; unused with systemd.
    pub pm2_file: String,
    /// The user PM2 or an app command runs as, never the agent's own user.
    #[serde(default)]
    pub user: String,
    /// The environment the app starts with.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// A file with more environment, relative to the release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_file: Option<String>,
}

/// The service's live state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentStatus {
    pub dir_exists: bool,
    pub current: Option<String>,
    /// `active`, `failed`, `missing` and the words systemd and PM2 share.
    pub active: String,
}

/// What to do to the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentServiceAction {
    Stop,
    Start,
    Restart,
}

/// `agent/service_control`: one change to the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentServiceControl {
    pub service: AgentService,
    pub action: AgentServiceAction,
}

/// How a release is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentReleaseKind {
    Open,
    Compressed,
}

/// One release on the server, as the agent sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentRelease {
    pub id: String,
    pub kind: AgentReleaseKind,
    pub current: bool,
    pub size_bytes: Option<u64>,
    /// Whether `backups/<id>.tar.zst` holds it.
    pub backup: bool,
    /// The text of `release.json`, when the folder has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}

/// `agent/releases_list`: one deployment folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleasesRequest {
    pub root: String,
}

/// Every release and backup in one deployment folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleases {
    pub current: Option<String>,
    pub entries: Vec<AgentRelease>,
}

/// `agent/link_shared`: link service.shared folders into a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLinkShared {
    pub root: String,
    pub release: String,
    /// Relative paths inside the release.
    pub shared: Vec<String>,
}

/// One path inside a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseFile {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub is_dir: bool,
}

/// What the agent list of release files depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseFilesRequest {
    pub root: String,
    pub release: String,
}

/// Every path of a release, without following links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseFiles {
    pub files: Vec<AgentReleaseFile>,
}

/// `agent/backup_create`: pack these paths of a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentBackupCreate {
    pub root: String,
    pub release: String,
    pub paths: Vec<String>,
}

/// The size of the backup that was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentBackupCreated {
    pub bytes: u64,
}

/// `agent/backup_restore`: unpack a release's backup beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentBackupRestore {
    pub root: String,
    pub release: String,
}

/// `agent/logs_follow`: the service's log, from its last lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLogsFollow {
    pub service: AgentService,
    pub lines: u32,
}

/// The agent's id for one followed log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLogsView {
    pub view: u64,
}

/// Stop following the log with this id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLogsStop {
    pub view: u64,
}

/// One line of a followed log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLogLine {
    pub view: u64,
    pub line: String,
}

/// A followed log ended on its own or because it was stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLogsEnded {
    pub view: u64,
    pub code: Option<u32>,
    pub error: Option<String>,
}

/// Remove exactly these unfinished files, which the owner approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCleanupWork {
    pub root: String,
    pub artifacts: Vec<String>,
}

/// What the approved cleanup removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCleaned {
    pub removed: Vec<String>,
}

/// Remove a failed release and the files sent to make it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentDiscard {
    pub root: String,
    pub release: String,
}

/// What one path inside a release is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentFileKind {
    Dir,
    File,
    Link,
}

/// One path inside a release, as the release screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentFileInfo {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub kind: AgentFileKind,
    /// As `ls -l` shows it, such as `-rw-r--r--`.
    pub mode: String,
    pub size: u64,
    pub target: Option<String>,
}

/// `agent/release_contents`: list a release folder or its backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseContentsRequest {
    pub root: String,
    pub release: String,
    /// Read `backups/<release>.tar.zst` instead of `releases/<release>`.
    pub backup: bool,
}

/// Every path of one release, and its `release.json` when read from a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseContents {
    pub files: Vec<AgentFileInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}

/// `agent/release_read_file`: the start of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseReadRequest {
    pub root: String,
    pub release: String,
    pub backup: bool,
    /// Relative to the release, `/`-separated.
    pub path: String,
    /// Stop after this many bytes.
    pub cap: u64,
}

/// The start of one file, as text when it is text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReleaseRead {
    pub text: String,
    pub binary: bool,
}

/// `agent/release_drop`: remove one opened release or one backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentDropRequest {
    pub root: String,
    pub release: String,
    /// Remove `releases/<release>`.
    pub open: bool,
    /// Remove `backups/<release>.tar.zst`.
    pub backup: bool,
}

/// Recent events for one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEventsRecent {
    pub project: String,
    pub limit: u32,
}

/// One event to save for a project and operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEventAppend {
    pub project: String,
    pub operation: String,
    pub kind: String,
    pub detail: String,
}

/// The saved event number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEventSaved {
    pub sequence: i64,
}

/// Read a bounded page of events after a sequence number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEventsAfter {
    /// Only this project's events; all projects when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub sequence: i64,
    pub limit: u32,
}

/// A numbered, persisted event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEvent {
    pub sequence: i64,
    pub project: String,
    pub operation: String,
    pub kind: String,
    pub detail: String,
    pub created_at: String,
}

/// One page of saved events, in sequence order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEvents {
    pub events: Vec<AgentEvent>,
}

/// A compressed release to receive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UploadPrepare {
    pub root: String,
    pub release: String,
    pub bytes: u64,
    pub sha256: String,
    /// The project lock this upload belongs to.
    pub lock_token: u64,
    /// True only when retrying this same operation after a reuse mismatch.
    #[serde(default)]
    pub retry: bool,
}

/// A private one-use reservation for the receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UploadReservation {
    pub reservation: String,
}

/// Files to compare with the current release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HashCurrent {
    /// Absolute deployment directory containing `current`.
    pub root: String,
    /// Relative file paths within the release.
    pub paths: Vec<String>,
}

/// SHA-256 hashes for files that exist and are safe to reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HashCurrentResult {
    pub hashes: BTreeMap<String, String>,
}

/// `agent/hello`: what the engine says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHello {
    /// The agent protocol the engine speaks.
    pub protocol: u32,
    /// The engine's version, such as `0.7.1`.
    pub version: String,
}

/// What `agent/hello` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHelloResult {
    /// The agent protocol the agent speaks.
    pub protocol: u32,
    /// The agent's version; its major version must equal the engine's.
    pub version: String,
    /// The server's operating system, such as `linux`.
    pub os: String,
    /// The server's CPU, such as `x86_64`.
    pub arch: String,
}

/// `agent/ping`: nothing to say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentPing {}

/// `agent/shutdown`: nothing to say; the request itself is the message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentShutdown {}
