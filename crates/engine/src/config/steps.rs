//! The twelve steps, their commands and their settings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::commands::CommandList;

/// The actions shipped out of a deploy.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
}

/// Step 1: checks before anything else.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreflightStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// Programs the server must have.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// Step 2: the build, and the file collection.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// The file whose version names the release.
    #[serde(default)]
    pub version_from: Option<String>,
}

/// Steps that only take commands.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
}

/// Step 5: what stays on the server once a release's folder goes.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BackupStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// How many old releases stay.
    #[serde(default)]
    pub keep: Option<Keep>,
    /// Paths setup made that it cannot make again.
    #[serde(default)]
    pub include: Vec<String>,
    /// Uploaded paths to leave out.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Step 9: the release must answer.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// The URL fetched on the server. It must answer 2xx.
    #[serde(default)]
    pub url: Option<String>,
    /// Seconds the release has to become healthy. Default: 20.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

impl Default for HealthStep {
    fn default() -> Self {
        Self {
            local: CommandList::default(),
            server: CommandList::default(),
            url: None,
            timeout_secs: default_timeout(),
        }
    }
}

/// Step 11: when putting an older release back.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RollbackStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// What an automatic rollback does. Default: ask.
    #[serde(default)]
    pub mode: Option<RollbackMode>,
    /// How long `ask` waits before rolling back. Default: 30.
    #[serde(default)]
    pub ask_secs: Option<u64>,
    /// Keep the failed release folder to inspect it. Default: false.
    #[serde(default)]
    pub keep_failed: Option<bool>,
}

/// What an automatic rollback does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RollbackMode {
    /// Ask whether to roll back; with no answer in time, roll back.
    #[default]
    Ask,
    /// Roll back at once.
    Auto,
    /// Never roll back on its own.
    Off,
}

/// Old releases kept on the server, for rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Keep {
    /// Releases kept unpacked and ready to switch to, counting the live one.
    /// Default: 5.
    #[serde(default = "default_open")]
    pub open: usize,
    /// Older releases kept only as their backup. 0 keeps none. Default: 15.
    #[serde(default = "default_compressed")]
    pub compressed: usize,
}

impl Default for Keep {
    fn default() -> Self {
        Self {
            open: default_open(),
            compressed: default_compressed(),
        }
    }
}

/// The release steps, with the settings each takes.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSteps {
    /// Step 1.
    #[serde(default)]
    pub preflight: PreflightStep,
    /// Step 2.
    #[serde(default)]
    pub build: BuildStep,
    /// Step 3.
    #[serde(default)]
    pub upload: Step,
    /// Step 4.
    #[serde(default)]
    pub setup: Step,
    /// Step 5.
    #[serde(default)]
    pub backup: BackupStep,
    /// Step 6.
    #[serde(default)]
    pub switch: Step,
    /// Step 7.
    #[serde(default)]
    pub restart: Step,
    /// Step 8.
    #[serde(default)]
    pub nginx: Step,
    /// Step 9.
    #[serde(default)]
    pub health: HealthStep,
    /// Step 10.
    #[serde(default)]
    pub done: Step,
    /// Step 11.
    #[serde(default)]
    pub rollback: RollbackStep,
    /// Step 12.
    #[serde(default)]
    pub failed: Step,
}

/// The app steps: the same commands, without the release's settings.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppSteps {
    /// Step 1.
    #[serde(default)]
    pub preflight: Step,
    /// Step 2.
    #[serde(default)]
    pub build: Step,
    /// Step 3.
    #[serde(default)]
    pub upload: Step,
    /// Step 4. Not allowed for a static app.
    #[serde(default)]
    pub setup: Step,
    /// Step 5. `include` is not allowed for a static app.
    #[serde(default)]
    pub backup: AppBackupStep,
    /// Step 6.
    #[serde(default)]
    pub switch: Step,
    /// Step 7.
    #[serde(default)]
    pub restart: Step,
    /// Step 8.
    #[serde(default)]
    pub nginx: Step,
    /// Step 9.
    #[serde(default)]
    pub health: AppHealthStep,
    /// Step 10.
    #[serde(default)]
    pub done: Step,
    /// Step 11.
    #[serde(default)]
    pub rollback: Step,
    /// Step 12.
    #[serde(default)]
    pub failed: Step,
}

/// An app's backup step: no `keep`, which belongs to the release.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppBackupStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// Paths setup made that it cannot make again.
    #[serde(default)]
    pub include: Vec<String>,
    /// Uploaded paths to leave out.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// An app's health step: `url` belongs to the app, `timeout_secs` to the release.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppHealthStep {
    /// On your PC.
    #[serde(default)]
    pub local: CommandList,
    /// On the server.
    #[serde(default)]
    pub server: CommandList,
    /// The URL fetched on the server. It must answer 2xx.
    #[serde(default)]
    pub url: Option<String>,
}

fn default_timeout() -> u64 {
    20
}

fn default_open() -> usize {
    5
}

fn default_compressed() -> usize {
    15
}
