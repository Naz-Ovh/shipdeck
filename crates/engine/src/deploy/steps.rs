//! The steps of a deploy or rollback: their names, keys and order.

use crate::config::DeployConfig;

/// Every step a deploy or rollback can have, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Connect,
    Preflight,
    Build,
    Pack,
    Upload,
    Unpack,
    Setup,
    Backup,
    Switch,
    Restart,
    Health,
    Done,
    Failed,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Self::Connect => "Connect",
            Self::Preflight => "Preflight",
            Self::Build => "Build",
            Self::Pack => "Pack",
            Self::Upload => "Upload",
            Self::Unpack => "Unpack",
            Self::Setup => "Setup",
            Self::Backup => "Backup",
            Self::Switch => "Switch",
            Self::Restart => "Restart",
            Self::Health => "Health",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }

    /// As deploy.yaml and `SHIPDECK_STEP` spell it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Preflight => "preflight",
            Self::Build => "build",
            Self::Pack => "pack",
            Self::Upload => "upload",
            Self::Unpack => "unpack",
            Self::Setup => "setup",
            Self::Backup => "backup",
            Self::Switch => "switch",
            Self::Restart => "restart",
            Self::Health => "health",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// The steps a deploy of `config` goes through. Failed is listed only when
/// `pipeline.failed` has commands.
pub fn deploy_steps(config: &DeployConfig) -> Vec<Step> {
    let mut steps = vec![
        Step::Preflight,
        Step::Build,
        Step::Pack,
        Step::Upload,
        Step::Unpack,
        Step::Setup,
        Step::Backup,
        Step::Switch,
        Step::Restart,
        Step::Health,
        Step::Done,
    ];
    if !config.pipeline.failed.run.local.is_empty() {
        steps.push(Step::Failed);
    }
    steps
}

/// The names of [`deploy_steps`], for the screen.
pub fn deploy_step_names(config: &DeployConfig) -> Vec<&'static str> {
    deploy_steps(config).into_iter().map(Step::name).collect()
}

/// What `SHIPDECK_ACTION` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Deploy,
    Rollback,
}

impl Action {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::Rollback => "rollback",
        }
    }
}
