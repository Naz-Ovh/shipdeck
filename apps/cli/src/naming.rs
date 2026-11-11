//! Which deploy a command runs on, and the two setup answers.

use std::process::ExitCode;

use anyhow::Result;
use shipdeck_protocol::message::DeployableInfo;
use shipdeck_protocol::message::request::SetupAccess;

use crate::AccessArg;
use crate::init;
use crate::target::Target;

/// Where the root file is, for a message that has to name it.
pub struct Where<'a> {
    /// The root file, or the name the folder would hold.
    pub root_file: &'a std::path::Path,
}

/// The deploy slugs a deploy command runs, in the order given.
///
/// A name must be one the root file lists; a file is the deploy the engine
/// read from it.
pub fn resolve_names(
    projects: &[DeployableInfo],
    where_: &Where<'_>,
    targets: &[Target],
) -> Result<Vec<String>, String> {
    if targets.is_empty() {
        return match projects {
            [one] => Ok(vec![one.slug.clone()]),
            [] => Err(format!("{} lists no deploys", where_.root_file.display())),
            _ => Err(format!("name the deploy to run: {}", listed(projects))),
        };
    }
    targets
        .iter()
        .map(|target| slug_of(projects, where_, target))
        .collect()
}

/// The deploy slug a command runs against.
pub fn resolve_name(
    projects: &[DeployableInfo],
    where_: &Where<'_>,
    target: Option<&Target>,
) -> Result<String, String> {
    match target {
        Some(target) => slug_of(projects, where_, target),
        None => match projects {
            [one] => Ok(one.slug.clone()),
            [] => Err(format!("{} lists no deploys", where_.root_file.display())),
            _ => Err(format!("name the deploy: {}", listed(projects))),
        },
    }
}

/// The slug of `target`: a name the root file lists, or the deploy a file
/// holds.
fn slug_of(
    projects: &[DeployableInfo],
    where_: &Where<'_>,
    target: &Target,
) -> Result<String, String> {
    match target {
        Target::Name(name) if projects.iter().any(|project| &project.slug == name) => {
            Ok(name.clone())
        }
        Target::Name(name) => Err(format!(
            "{} has no deploy named {name}; it lists {}. A deploy file's name ends in \
             .yaml or .yml",
            where_.root_file.display(),
            listed(projects)
        )),
        Target::File(path) => {
            let wanted = path.display().to_string();
            projects
                .iter()
                .find(|project| project.folder == wanted)
                .map(|project| project.slug.clone())
                .ok_or_else(|| format!("{wanted} has no deploy the engine could read"))
        }
    }
}

/// The deploy slugs, for a message that asks the reader to name one.
fn listed(projects: &[DeployableInfo]) -> String {
    projects
        .iter()
        .map(|project| project.slug.as_str())
        .collect::<Vec<_>>()
        .join(", ")
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
