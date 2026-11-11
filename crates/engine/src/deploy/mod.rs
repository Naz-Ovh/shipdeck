//! A deploy and a rollback: one pipeline over one deployment, split by step.

mod activate;
mod backup;
mod build;
pub(crate) mod commands;
mod preflight;
mod recover;
pub mod report;
mod run;
mod steps;
mod upload;

pub use report::{
    LogKind, OpEvent, OpId, OpUpdate, Outcome, Reporter, RollbackQuestion, StepState,
    UnchangedQuestion, Unit,
};
pub use run::{deploy, deploy_plan, rollback, rollback_plan};
pub use steps::{
    Action, Step, app_commands, deploy_steps, release_commands, rollback_steps, scopes,
};

/// The first line of an error, short enough for a step's detail column.
pub(crate) fn short(err: &anyhow::Error) -> String {
    let text = err.to_string();
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 70 {
        format!("{}…", line.chars().take(69).collect::<String>())
    } else {
        line.to_string()
    }
}
