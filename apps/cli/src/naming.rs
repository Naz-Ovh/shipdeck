//! Which deploy a command runs on, and the two setup answers.

use std::process::ExitCode;

use anyhow::Result;
use shipdeck_engine::config::Env;
use shipdeck_protocol::message::DeployableInfo;
use shipdeck_protocol::message::request::SetupAccess;

use crate::AccessArg;
use crate::init;
use crate::target::Target;

/// Where the root file is, for a message that has to name it.
pub struct Where<'a> {
    /// The project folder, which a relative deploy file starts from.
    pub root: &'a std::path::Path,
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
            [one] => {
                slug_of(projects, where_, &Target::Name(one.slug.clone())).map(|slug| vec![slug])
            }
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
            [one] => slug_of(projects, where_, &Target::Name(one.slug.clone())),
            [] => Err(format!("{} lists no deploys", where_.root_file.display())),
            _ => Err(format!("name the deploy: {}", listed(projects))),
        },
    }
}

/// The slug of `target`, unless that deploy is unavailable: then why.
fn slug_of(
    projects: &[DeployableInfo],
    where_: &Where<'_>,
    target: &Target,
) -> Result<String, String> {
    let slug = find_slug(projects, where_, target)?;
    match projects
        .iter()
        .find(|project| project.slug == slug)
        .and_then(|project| project.unavailable.as_deref())
    {
        Some(reason) => Err(format!("the deploy {slug} is unavailable: {reason}")),
        None => Ok(slug),
    }
}

/// The slug of `target`: a name the root file lists, or the deploy a file
/// holds.
fn find_slug(
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
            let Target::File(wanted) = target.resolved(where_.root) else {
                unreachable!("a file target resolves to a file");
            };
            projects
                .iter()
                .find(|project| std::path::Path::new(&project.folder) == wanted)
                .map(|project| project.slug.clone())
                .ok_or_else(|| format!("{} has no deploy the engine could read", path.display()))
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
        // A problem of the whole file, or of the run, has no line.
        if problem.line == 0 {
            eprintln!("{}: {}", problem.file, problem.message);
        } else {
            eprintln!(
                "{}:{}:{}: {}",
                problem.file, problem.line, problem.column, problem.message
            );
        }
        if let Some(hint) = &problem.hint {
            eprintln!("  {hint}");
        }
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
pub fn setup_password(given: Option<String>, env: &Env) -> Result<Option<String>> {
    if given.is_some() {
        return Ok(given);
    }
    if let Some(password) = env.get("SHIPDECK_SUDO_PASSWORD") {
        return Ok(Some(password.to_string()));
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
