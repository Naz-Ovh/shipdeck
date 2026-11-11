//! The twelve steps of a deploy and a rollback, their names and order.

use crate::config::{AppSteps, CommandList, Deployment, ReleaseSteps};

/// Every step a deploy or rollback can have, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Preflight,
    Build,
    Upload,
    Setup,
    Backup,
    Switch,
    Restart,
    Nginx,
    Health,
    Done,
    Rollback,
    Failed,
}

impl Step {
    /// Every step, in the design's order.
    pub const ALL: [Step; 12] = [
        Step::Preflight,
        Step::Build,
        Step::Upload,
        Step::Setup,
        Step::Backup,
        Step::Switch,
        Step::Restart,
        Step::Nginx,
        Step::Health,
        Step::Done,
        Step::Rollback,
        Step::Failed,
    ];

    /// The name a screen shows.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Preflight => "Preflight",
            Self::Build => "Build",
            Self::Upload => "Upload",
            Self::Setup => "Setup",
            Self::Backup => "Backup",
            Self::Switch => "Switch",
            Self::Restart => "Restart",
            Self::Nginx => "Nginx",
            Self::Health => "Health",
            Self::Done => "Done",
            Self::Rollback => "Rollback",
            Self::Failed => "Failed",
        }
    }

    /// The key a deploy file and `SHIPDECK_STEP` use.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Build => "build",
            Self::Upload => "upload",
            Self::Setup => "setup",
            Self::Backup => "backup",
            Self::Switch => "switch",
            Self::Restart => "restart",
            Self::Nginx => "nginx",
            Self::Health => "health",
            Self::Done => "done",
            Self::Rollback => "rollback",
            Self::Failed => "failed",
        }
    }
}

/// The steps a deploy goes through.
#[must_use]
pub fn deploy_steps() -> Vec<Step> {
    Step::ALL.to_vec()
}

/// The steps a rollback goes through.
#[must_use]
pub fn rollback_steps() -> Vec<Step> {
    vec![
        Step::Preflight,
        Step::Rollback,
        Step::Setup,
        Step::Switch,
        Step::Restart,
        Step::Nginx,
        Step::Health,
        Step::Done,
        Step::Failed,
    ]
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

/// The commands of one step at one scope.
#[derive(Debug, Clone, Copy)]
pub struct Commands<'a> {
    pub app: Option<&'a str>,
    pub local: &'a CommandList,
    pub server: &'a CommandList,
}

/// The release commands of a step.
#[must_use]
pub fn release_commands(steps: &ReleaseSteps, step: Step) -> (&CommandList, &CommandList) {
    match step {
        Step::Preflight => (&steps.preflight.local, &steps.preflight.server),
        Step::Build => (&steps.build.local, &steps.build.server),
        Step::Upload => (&steps.upload.local, &steps.upload.server),
        Step::Setup => (&steps.setup.local, &steps.setup.server),
        Step::Backup => (&steps.backup.local, &steps.backup.server),
        Step::Switch => (&steps.switch.local, &steps.switch.server),
        Step::Restart => (&steps.restart.local, &steps.restart.server),
        Step::Nginx => (&steps.nginx.local, &steps.nginx.server),
        Step::Health => (&steps.health.local, &steps.health.server),
        Step::Done => (&steps.done.local, &steps.done.server),
        Step::Rollback => (&steps.rollback.local, &steps.rollback.server),
        Step::Failed => (&steps.failed.local, &steps.failed.server),
    }
}

/// The commands of one app's step.
#[must_use]
pub fn app_commands(steps: &AppSteps, step: Step) -> (&CommandList, &CommandList) {
    match step {
        Step::Preflight => (&steps.preflight.local, &steps.preflight.server),
        Step::Build => (&steps.build.local, &steps.build.server),
        Step::Upload => (&steps.upload.local, &steps.upload.server),
        Step::Setup => (&steps.setup.local, &steps.setup.server),
        Step::Backup => (&steps.backup.local, &steps.backup.server),
        Step::Switch => (&steps.switch.local, &steps.switch.server),
        Step::Restart => (&steps.restart.local, &steps.restart.server),
        Step::Nginx => (&steps.nginx.local, &steps.nginx.server),
        Step::Health => (&steps.health.local, &steps.health.server),
        Step::Done => (&steps.done.local, &steps.done.server),
        Step::Rollback => (&steps.rollback.local, &steps.rollback.server),
        Step::Failed => (&steps.failed.local, &steps.failed.server),
    }
}

/// The release's commands, then each app's, in name order.
#[must_use]
pub fn scopes(deployment: &Deployment, step: Step) -> Vec<Commands<'_>> {
    let (local, server) = release_commands(deployment.steps(), step);
    let mut out = Vec::with_capacity(deployment.deploy.apps.len() + 1);
    out.push(Commands {
        app: None,
        local,
        server,
    });
    for (name, app) in &deployment.deploy.apps {
        let (local, server) = app_commands(&app.steps, step);
        out.push(Commands {
            app: Some(name),
            local,
            server,
        });
    }
    out
}

/// Every scope of every step, for the preflight runner check.
#[must_use]
pub fn all_scopes(deployment: &Deployment) -> Vec<(Step, Commands<'_>)> {
    let mut out = Vec::new();
    for step in Step::ALL {
        for scope in scopes(deployment, step) {
            out.push((step, scope));
        }
    }
    out
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
