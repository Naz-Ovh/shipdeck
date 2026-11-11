//! Which deploy a command runs on, and the two setup answers.

use std::process::ExitCode;

use anyhow::Result;
use shipdeck_protocol::message::DeployableInfo;
use shipdeck_protocol::message::request::SetupAccess;

use crate::AccessArg;
use crate::init;

/// The deploy names a deploy command runs.
pub fn resolve_names(
    projects: &[DeployableInfo],
    names: Vec<String>,
) -> Result<Vec<String>, String> {
    if !names.is_empty() {
        return Ok(names);
    }
    match projects {
        [one] => Ok(vec![one.slug.clone()]),
        [] => Err("no deploy to run".into()),
        _ => Err(format!(
            "name the deploy to run: {}",
            projects
                .iter()
                .map(|project| project.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The deploy name a command runs against.
pub fn resolve_name(projects: &[DeployableInfo], name: Option<&str>) -> Result<String, String> {
    if let Some(name) = name {
        return Ok(name.to_string());
    }
    match projects {
        [one] => Ok(one.slug.clone()),
        [] => Err("the project has no deploys".into()),
        _ => Err(format!(
            "name the deploy: {}",
            projects
                .iter()
                .map(|project| project.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Prints an error and ends as a failure.
pub fn fail(error: &str) -> ExitCode {
    eprintln!("✘ {error}");
    ExitCode::FAILURE
}

/// What the engine found in a file this program let through: say where, and
/// stop. Every deploy is refused, as the check above does.
pub fn problems(found: &[shipdeck_protocol::message::Problem]) -> ExitCode {
    for problem in found {
        eprintln!(
            "{}:{}:{}: {}",
            problem.file, problem.line, problem.column, problem.message
        );
    }
    ExitCode::FAILURE
}

/// `--access`, or the question when there is someone to ask.
pub fn setup_access(given: Option<AccessArg>) -> Result<SetupAccess> {
    if let Some(given) = given {
        return Ok(given.into());
    }
    if !init::can_ask() {
        return Ok(SetupAccess::Commands);
    }
    let chosen = inquire::Select::new(
        "Sudo access:",
        vec![
            "Commands only (password-free sudo for what shipdeck runs)",
            "Full sudo (password-free sudo for every command)",
        ],
    )
    .prompt()
    .map_err(|err| anyhow::anyhow!("no access chosen: {err}"))?;
    Ok(if chosen.starts_with("Full") {
        SetupAccess::Full
    } else {
        SetupAccess::Commands
    })
}

/// `--sudo-password`, `SHIPDECK_SUDO_PASSWORD`, or the hidden question.
pub fn setup_password(given: Option<String>) -> Result<Option<String>> {
    if given.is_some() {
        return Ok(given);
    }
    if let Some(password) = std::env::var("SHIPDECK_SUDO_PASSWORD")
        .ok()
        .filter(|password| !password.is_empty())
    {
        return Ok(Some(password));
    }
    if !init::can_ask() {
        return Ok(None);
    }
    let password = inquire::Password::new("Sudo password (blank when sudo needs none):")
        .without_confirmation()
        .prompt()
        .map_err(|err| anyhow::anyhow!("no sudo password entered: {err}"))?;
    Ok((!password.is_empty()).then_some(password))
}
