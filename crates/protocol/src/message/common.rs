//! Values several messages share: how to log in, what a step is doing,
//! what a release holds.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How to log in to the server, as `workspace/open` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Login {
    /// The `IdentityFile` from `~/.ssh/config`; else ask for a password.
    Auto,
    /// A private key file on this PC.
    Key { path: String },
    /// A private key's text, such as a CI secret.
    KeyText { text: String },
    /// A password given up front, so nothing is asked.
    Password { password: String },
}

/// What to do when a release fails after it went live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    /// Put the release that was live before back at once.
    Rollback,
    /// Keep the failed release, and ask before going back.
    Keep,
}

/// What `service/control` should do to the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Stop,
    Restart,
}

/// Whether a release is unpacked, or only kept as its backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    /// Unpacked in `releases/`, ready to switch to.
    Open,
    /// Only its tarball in `backups/`.
    Compressed,
}

/// Where a release's files are read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseSource {
    /// `releases/<id>/`.
    Folder,
    /// `backups/<id>.tar.zst`.
    Backup,
}

/// What one path of a release is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Dir,
    File,
    Link,
}

/// What a step of a deploy or rollback is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Done,
    /// It finished with something worth reading.
    Warned,
    Failed,
    Skipped,
}

/// What a progress count measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Bytes,
    Items,
    Seconds,
}

/// What one line of an operation's log is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    /// A step starts.
    Step,
    /// A command is about to run, here or on the server.
    Command,
    Out,
    Err,
    Info,
    Success,
    Warn,
    Error,
}

/// How a deploy or rollback ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// The new release is live.
    Succeeded { release: String },
    /// It stopped before the live release was touched.
    Failed { error: String },
    /// The new release failed, and the previous one is live again.
    RolledBack { error: String, to: String },
    /// The new release failed, and putting the old one back failed too.
    RollbackFailed {
        error: String,
        rollback_error: String,
    },
    /// The new release failed after the switch, and you chose to keep it.
    Kept { release: String, error: String },
    /// The release held what the live one holds, and you chose not to deploy it.
    Unchanged { live: String },
}

/// One project in the workspace, as `workspace/open` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeployableInfo {
    /// The folder's name; what every other message calls the project.
    pub slug: String,
    /// The `name` from `deploy.yaml`.
    pub name: String,
    /// `ssh.host` from `deploy.yaml`: an alias or an address.
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The port the server answers on.
    #[serde(default = "port")]
    pub port: u16,
    /// `service.dir`: where the project lives on the server.
    #[serde(default)]
    pub dir: String,
    /// The project's folder on this PC, where its `deploy.yaml` is.
    #[serde(default)]
    pub folder: String,
}

/// SSH's own port, for a project that does not say.
fn port() -> u16 {
    22
}

/// One mistake in a `deploy.yaml`, at its line and column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    /// The `deploy.yaml` path, as it was opened.
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
    /// Printed after the carets, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// How to put it right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The parts of a release id, so a screen can sort and show them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseName {
    pub version: String,
    pub git: String,
    pub dirty: bool,
    /// When it was built, as RFC 3339 in UTC.
    #[serde(default)]
    pub built_at: String,
}

/// One release on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseInfo {
    pub id: String,
    /// `None` for a folder shipdeck did not name; it is never touched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ReleaseName>,
    pub kind: ReleaseKind,
    /// It is what `current` points to.
    pub current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// `backups/<id>.tar.zst` holds it.
    pub backup: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Manifest>,
}

/// The `release.json` each release carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub git: String,
    pub dirty: bool,
    /// When it was built, as RFC 3339 in UTC.
    pub built_at: String,
    pub deployed_by: String,
    /// `sha256:<hex>` over what the release holds and how it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// One path in a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileInfo {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub kind: FileKind,
    /// As `ls -l` shows it, such as `-rw-r--r--`.
    pub mode: String,
    pub size: u64,
    /// Where a link points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}
