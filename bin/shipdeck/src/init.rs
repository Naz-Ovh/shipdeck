//! `shipdeck init`: asks in the terminal for whatever the flags left out,
//! then has the engine check and write the new deployable folder.

use std::fmt;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use inquire::error::InquireError;
use inquire::validator::Validation;
use inquire::{Select, Text};

use shipdeck_engine::config::{self, Manager};
use shipdeck_engine::init::{self, NewDeployable, Preset};
use shipdeck_engine::ssh::ssh_config;

/// What the flags gave.
pub struct Given {
    pub root: PathBuf,
    pub name: Option<String>,
    pub preset: Option<Preset>,
    pub manager: Option<Manager>,
    pub host: Option<String>,
    pub user: Option<String>,
    pub project: Option<PathBuf>,
}

/// Whether there is someone to ask: prompts read stdin and draw on stderr.
pub fn can_ask() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

pub fn run(given: Given) -> Result<ExitCode> {
    if !given.root.is_dir() {
        bail!("{} is not a folder; create it first", given.root.display());
    }
    let root = given.root.clone();
    let new = match answers(&root, given) {
        Ok(new) => new,
        Err(Stop::Cancelled) => {
            eprintln!("Nothing written.");
            return Ok(ExitCode::FAILURE);
        }
        Err(Stop::Interrupted) => return Ok(ExitCode::from(130)),
        Err(Stop::Failed(err)) => return Err(err),
    };
    let yaml = match init::check(&root, &new) {
        Ok(yaml) => yaml,
        Err(problems) => {
            let base = std::env::current_dir().unwrap_or_default();
            let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
            eprint!(
                "{}",
                config::render(&[problems], &base, color, "nothing written")
            );
            return Ok(ExitCode::FAILURE);
        }
    };
    for path in init::write(&root, &new.name, &yaml)? {
        println!("wrote {}", path.display());
    }
    println!(
        "Fill in {}, then run: shipdeck {}",
        root.join(&new.name).join(".env").display(),
        root.display()
    );
    Ok(ExitCode::SUCCESS)
}

enum Stop {
    /// Esc on a question.
    Cancelled,
    /// Ctrl-C on a question.
    Interrupted,
    Failed(anyhow::Error),
}

impl From<InquireError> for Stop {
    fn from(err: InquireError) -> Self {
        match err {
            InquireError::OperationCanceled => Stop::Cancelled,
            InquireError::OperationInterrupted => Stop::Interrupted,
            err => Stop::Failed(err.into()),
        }
    }
}

impl From<anyhow::Error> for Stop {
    fn from(err: anyhow::Error) -> Self {
        Stop::Failed(err)
    }
}

fn answers(root: &Path, given: Given) -> Result<NewDeployable, Stop> {
    let name = match given.name {
        Some(name) => {
            if let Some(problem) = init::name_problem(root, &name) {
                return Err(anyhow::anyhow!("{name:?}: {problem}").into());
            }
            name
        }
        None => {
            let folder = root.to_path_buf();
            Text::new("Name:")
                .with_help_message("the new folder and the service, such as my-api")
                .with_validator(move |name: &str| {
                    Ok(match init::name_problem(&folder, name) {
                        Some(problem) => Validation::Invalid(problem.into()),
                        None => Validation::Valid,
                    })
                })
                .prompt()?
        }
    };
    let preset = match given.preset {
        Some(preset) => preset,
        None => {
            Select::new("Preset:", Preset::ALL.map(Shown).to_vec())
                .prompt()?
                .0
        }
    };
    let manager = match given.manager {
        Some(manager) => manager,
        None if can_ask() => {
            Select::new("Service manager:", MANAGERS.map(Shown).to_vec())
                .prompt()?
                .0
        }
        None => Manager::Systemd,
    };
    let host = match given.host {
        Some(host) => host,
        None => Text::new("SSH host:")
            .with_help_message("an address, or an alias from ~/.ssh/config")
            .with_validator(|host: &str| {
                Ok(if host.trim().is_empty() {
                    Validation::Invalid("the server to deploy to".into())
                } else {
                    Validation::Valid
                })
            })
            .prompt()?
            .trim()
            .to_string(),
    };
    let user = match given.user {
        Some(user) => Some(user),
        None if can_ask() => ask_user(&host)?,
        None => None,
    };
    let project = match given.project {
        Some(project) => project,
        None if can_ask() => PathBuf::from(
            Text::new("Project folder:")
                .with_help_message("your source, from where you are now")
                .with_default(".")
                .with_validator(|path: &str| {
                    Ok(if Path::new(path).is_dir() {
                        Validation::Valid
                    } else {
                        Validation::Invalid("not a folder".into())
                    })
                })
                .prompt()?,
        ),
        None => PathBuf::from("."),
    };
    let project = init::project_from(root, &name, &project)?;
    Ok(NewDeployable {
        name,
        preset,
        manager,
        host,
        user,
        project,
    })
}

/// The SSH user, or `None` to use the alias's `User`.
fn ask_user(host: &str) -> Result<Option<String>, Stop> {
    let from_alias = ssh_config::lookup(host).user;
    let mut question = Text::new("SSH user:");
    if let Some(user) = &from_alias {
        question = question
            .with_default(user)
            .with_help_message("the default comes from ~/.ssh/config");
    }
    let user = question
        .with_validator(|user: &str| {
            Ok(if user.trim().is_empty() {
                Validation::Invalid("the user shipdeck logs in and deploys as".into())
            } else {
                Validation::Valid
            })
        })
        .prompt()?
        .trim()
        .to_string();
    Ok((from_alias.as_deref() != Some(&user)).then_some(user))
}

const MANAGERS: [Manager; 2] = [Manager::Systemd, Manager::Pm2];

/// A choice in a list, with a few words on what it means.
#[derive(Clone, Copy)]
struct Shown<T>(T);

impl fmt::Display for Shown<Preset> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:<7} {}", self.0.name(), self.0.summary())
    }
}

impl fmt::Display for Shown<Manager> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Manager::Systemd => f.write_str("systemd  a unit installed and restarted with sudo"),
            Manager::Pm2 => f.write_str("pm2      runs as the SSH user, no sudo"),
        }
    }
}
