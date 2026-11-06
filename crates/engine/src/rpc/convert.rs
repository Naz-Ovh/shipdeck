//! The engine's own types, mapped onto the protocol's.
//!
//! The wire is a contract, so the two never meet by accident: a refactor here
//! changes no message.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use shipdeck_protocol::message::common as wire;
use shipdeck_protocol::message::request::{ReleasesListResult, StatusGetResult};
use shipdeck_protocol::wire::{ErrorObject, code};

use crate::config::{Deployable, FileProblems};
use crate::deploy::Step;
use crate::deploy::report;
use crate::release;
use crate::remote::{self, Control, Listing, ReleaseContents, Status};
use crate::ssh::Login;

/// A method's answer, before it is written.
pub type Reply = Result<Value, ErrorObject>;

/// The params of a request, or `INVALID_PARAMS` when they do not fit.
///
/// # Errors
/// Returns `INVALID_PARAMS` when the value is not the shape the method takes.
pub fn params<P: DeserializeOwned>(value: Value) -> Result<P, ErrorObject> {
    serde_json::from_value(value).map_err(|err| {
        ErrorObject::new(
            code::INVALID_PARAMS,
            format!("the params do not fit this method: {err}"),
        )
    })
}

/// An answer as the wire carries it.
///
/// # Errors
/// Returns `INTERNAL_ERROR` when the type does not write as JSON.
pub fn value<R: Serialize>(answer: &R) -> Reply {
    serde_json::to_value(answer).map_err(|err| {
        ErrorObject::new(
            code::INTERNAL_ERROR,
            format!("the answer does not write as JSON: {err}"),
        )
    })
}

/// `UNKNOWN_DEPLOYABLE` for a name that is not in the workspace.
pub fn unknown_deployable(slug: &str) -> ErrorObject {
    ErrorObject::new(
        code::UNKNOWN_DEPLOYABLE,
        format!("there is no project named {slug:?} in this workspace"),
    )
}

/// A failure the client cannot put right: the message says what happened.
pub fn failed(err: &anyhow::Error) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, format!("{err:#}"))
}

/// The engine's login as the app sends it.
pub fn login(login: wire::Login) -> Login {
    match login {
        wire::Login::Auto => Login::Auto,
        wire::Login::Key { path } => Login::Key(path.into()),
        wire::Login::KeyText { text } => Login::KeyText(text),
        wire::Login::Password { password } => Login::Password(password),
    }
}

pub fn deployable_info(deployable: &Deployable) -> wire::DeployableInfo {
    wire::DeployableInfo {
        slug: deployable.slug.clone(),
        name: deployable.config.name.clone(),
        host: deployable.endpoint.name.clone(),
        user: Some(deployable.endpoint.user.clone()),
    }
}

/// One entry per problem, so a screen can point at its line.
pub fn problems(found: &FileProblems) -> Vec<wire::Problem> {
    found
        .problems
        .iter()
        .map(|problem| wire::Problem {
            file: found.path.display().to_string(),
            line: index(problem.at.map_or(0, |at| at.start.0)),
            column: index(problem.at.map_or(0, |at| at.start.1)),
            message: problem.message.clone(),
            label: (!problem.label.is_empty()).then(|| problem.label.clone()),
            hint: problem.help.clone(),
        })
        .collect()
}

pub fn status(state: &Status) -> StatusGetResult {
    StatusGetResult {
        dir_exists: state.dir_exists,
        current: state.current.clone(),
        state: state.active.clone(),
    }
}

pub fn listing(found: &Listing) -> ReleasesListResult {
    ReleasesListResult {
        current: found.current.clone(),
        entries: found.entries.iter().map(release_info).collect(),
    }
}

fn release_info(entry: &remote::ReleaseEntry) -> wire::ReleaseInfo {
    wire::ReleaseInfo {
        id: entry.id.clone(),
        name: entry.name.as_ref().map(release_name),
        kind: kind(entry.kind),
        current: entry.current,
        size_bytes: entry.size_bytes,
        backup: entry.backup,
        manifest: entry.manifest.as_ref().map(manifest),
    }
}

fn kind(kind: remote::Kind) -> wire::ReleaseKind {
    match kind {
        remote::Kind::Open => wire::ReleaseKind::Open,
        remote::Kind::Compressed => wire::ReleaseKind::Compressed,
    }
}

fn release_name(name: &release::ReleaseName) -> wire::ReleaseName {
    wire::ReleaseName {
        version: name.version.clone(),
        git: name.git.clone(),
        dirty: name.dirty,
    }
}

fn manifest(manifest: &release::Manifest) -> wire::Manifest {
    wire::Manifest {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        git: manifest.git.clone(),
        dirty: manifest.dirty,
        built_at: manifest.built_at.clone(),
        deployed_by: manifest.deployed_by.clone(),
        content: manifest.content.clone(),
    }
}

pub fn contents(
    found: &ReleaseContents,
) -> shipdeck_protocol::message::request::ReleaseContentsResult {
    shipdeck_protocol::message::request::ReleaseContentsResult {
        files: found.files.iter().map(file_info).collect(),
        manifest: found.manifest.as_ref().map(manifest),
    }
}

fn file_info(info: &remote::FileInfo) -> wire::FileInfo {
    wire::FileInfo {
        path: info.path.clone(),
        kind: file_kind(info.kind),
        mode: info.mode.clone(),
        size: info.size,
        target: info.target.clone(),
    }
}

fn file_kind(kind: remote::FileKind) -> wire::FileKind {
    match kind {
        remote::FileKind::Dir => wire::FileKind::Dir,
        remote::FileKind::File => wire::FileKind::File,
        remote::FileKind::Link => wire::FileKind::Link,
    }
}

/// Where a release's files are read from: a folder or a backup.
pub fn source(source: wire::ReleaseSource) -> remote::Source {
    match source {
        wire::ReleaseSource::Folder => remote::Source::Folder,
        wire::ReleaseSource::Backup => remote::Source::Backup,
    }
}

/// What `service/control` does to the service.
pub fn control(action: wire::Action) -> Control {
    match action {
        wire::Action::Start => Control::Start,
        wire::Action::Stop => Control::Stop,
        wire::Action::Restart => Control::Restart,
    }
}

pub fn step_state(state: report::StepState) -> wire::StepState {
    match state {
        report::StepState::Pending => wire::StepState::Pending,
        report::StepState::Running => wire::StepState::Running,
        report::StepState::Done => wire::StepState::Done,
        report::StepState::Warned => wire::StepState::Warned,
        report::StepState::Failed => wire::StepState::Failed,
        report::StepState::Skipped => wire::StepState::Skipped,
    }
}

pub fn unit(unit: report::Unit) -> wire::Unit {
    match unit {
        report::Unit::Bytes => wire::Unit::Bytes,
        report::Unit::Items => wire::Unit::Items,
        report::Unit::Seconds => wire::Unit::Seconds,
    }
}

pub fn log_kind(kind: report::LogKind) -> wire::LogKind {
    match kind {
        report::LogKind::Step => wire::LogKind::Step,
        report::LogKind::Command => wire::LogKind::Command,
        report::LogKind::Out => wire::LogKind::Out,
        report::LogKind::Err => wire::LogKind::Err,
        report::LogKind::Info => wire::LogKind::Info,
        report::LogKind::Success => wire::LogKind::Success,
        report::LogKind::Warn => wire::LogKind::Warn,
        report::LogKind::Error => wire::LogKind::Error,
    }
}

pub fn outcome(outcome: &report::Outcome) -> wire::Outcome {
    match outcome {
        report::Outcome::Succeeded { release } => wire::Outcome::Succeeded {
            release: release.clone(),
        },
        report::Outcome::Failed { error } => wire::Outcome::Failed {
            error: error.clone(),
        },
        report::Outcome::RolledBack { error, to } => wire::Outcome::RolledBack {
            error: error.clone(),
            to: to.clone(),
        },
        report::Outcome::RollbackFailed {
            error,
            rollback_error,
        } => wire::Outcome::RollbackFailed {
            error: error.clone(),
            rollback_error: rollback_error.clone(),
        },
        report::Outcome::Kept { release, error } => wire::Outcome::Kept {
            release: release.clone(),
            error: error.clone(),
        },
        report::Outcome::Unchanged { live } => wire::Outcome::Unchanged { live: live.clone() },
    }
}

/// The steps a run goes through, by the keys `deploy.yaml` knows.
pub fn steps(steps: &[Step]) -> Vec<String> {
    steps.iter().map(|step| step.key().to_string()).collect()
}

/// A place in a list, such as a step's or a line's: a plan is never long
/// enough for `u32` to lose it.
pub fn index(number: usize) -> u32 {
    u32::try_from(number).unwrap_or(u32::MAX)
}
