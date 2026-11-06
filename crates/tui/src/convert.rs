//! What the engine sends, in the types the screens already draw.
//!
//! The wire is the contract; these are the shapes the TUI was built on.

use shipdeck_protocol::message::common as wire;
use shipdeck_protocol::message::event::Event;
use shipdeck_protocol::message::request::{
    ReleaseContentsResult, ReleaseReadFileResult, ReleasesListResult, StatusGetResult,
};

use shipdeck_engine::deploy::report::{self, OpEvent, OpUpdate};
use shipdeck_engine::release::{Manifest, ReleaseName};
use shipdeck_engine::remote::{
    self, Control, FileInfo, FileText, Kind, Listing, ReleaseContents, ReleaseEntry, Source, Status,
};

/// One event of a running operation, or `None` for the two log events, which
/// the view takes on its own.
pub fn update(event: Event) -> Option<OpUpdate> {
    let op = match &event {
        Event::Step(step) => step.op,
        Event::Progress(progress) => progress.op,
        Event::Log(log) => log.op,
        Event::Release(release) => release.op,
        Event::Finished(finished) => finished.op,
        Event::LogsLine(_) | Event::LogsEnded(_) => return None,
    };
    let event = match event {
        Event::Step(step) => OpEvent::Step {
            step: index(step.step),
            state: step_state(step.state),
            detail: step.detail,
        },
        Event::Progress(progress) => OpEvent::Progress {
            step: index(progress.step),
            done: progress.done,
            total: progress.total,
            unit: unit(progress.unit),
            label: progress.label,
        },
        Event::Log(log) => OpEvent::Log {
            kind: log_kind(log.kind),
            text: log.text,
        },
        Event::Release(release) => OpEvent::Release(release.release),
        Event::Finished(finished) => OpEvent::Finished(outcome(finished.outcome)),
        Event::LogsLine(_) | Event::LogsEnded(_) => return None,
    };
    Some(OpUpdate { op, event })
}

pub fn status(state: StatusGetResult) -> Status {
    Status {
        dir_exists: state.dir_exists,
        current: state.current,
        active: state.state,
    }
}

pub fn listing(found: ReleasesListResult) -> Listing {
    Listing {
        current: found.current,
        entries: found.entries.into_iter().map(entry).collect(),
    }
}

fn entry(info: wire::ReleaseInfo) -> ReleaseEntry {
    // The id carries the release's parts; the engine sent them apart only for
    // screens that cannot read an id.
    let name = ReleaseName::parse(&info.id);
    ReleaseEntry {
        id: info.id,
        name,
        kind: match info.kind {
            wire::ReleaseKind::Open => Kind::Open,
            wire::ReleaseKind::Compressed => Kind::Compressed,
        },
        current: info.current,
        size_bytes: info.size_bytes,
        backup: info.backup,
        manifest: info.manifest.map(manifest),
    }
}

fn manifest(found: wire::Manifest) -> Manifest {
    Manifest {
        id: found.id,
        version: found.version,
        git: found.git,
        dirty: found.dirty,
        built_at: found.built_at,
        deployed_by: found.deployed_by,
        content: found.content,
    }
}

pub fn contents(found: ReleaseContentsResult) -> ReleaseContents {
    ReleaseContents {
        files: found.files.into_iter().map(file_info).collect(),
        manifest: found.manifest.map(manifest),
    }
}

fn file_info(info: wire::FileInfo) -> FileInfo {
    FileInfo {
        path: info.path,
        kind: match info.kind {
            wire::FileKind::Dir => remote::FileKind::Dir,
            wire::FileKind::File => remote::FileKind::File,
            wire::FileKind::Link => remote::FileKind::Link,
        },
        mode: info.mode,
        size: info.size,
        target: info.target,
    }
}

pub fn file_text(found: ReleaseReadFileResult) -> FileText {
    FileText {
        text: found.text,
        binary: found.binary,
    }
}

/// The engine reads a release from its folder or from its backup.
pub fn source(source: Source) -> wire::ReleaseSource {
    match source {
        Source::Folder => wire::ReleaseSource::Folder,
        Source::Backup => wire::ReleaseSource::Backup,
    }
}

/// What `service/control` does to the service.
pub fn action(action: Control) -> wire::Action {
    match action {
        Control::Stop => wire::Action::Stop,
        Control::Start => wire::Action::Start,
        Control::Restart => wire::Action::Restart,
    }
}

fn step_state(state: wire::StepState) -> report::StepState {
    match state {
        wire::StepState::Pending => report::StepState::Pending,
        wire::StepState::Running => report::StepState::Running,
        wire::StepState::Done => report::StepState::Done,
        wire::StepState::Warned => report::StepState::Warned,
        wire::StepState::Failed => report::StepState::Failed,
        wire::StepState::Skipped => report::StepState::Skipped,
    }
}

fn unit(unit: wire::Unit) -> report::Unit {
    match unit {
        wire::Unit::Bytes => report::Unit::Bytes,
        wire::Unit::Items => report::Unit::Items,
        wire::Unit::Seconds => report::Unit::Seconds,
    }
}

fn log_kind(kind: wire::LogKind) -> report::LogKind {
    match kind {
        wire::LogKind::Step => report::LogKind::Step,
        wire::LogKind::Command => report::LogKind::Command,
        wire::LogKind::Out => report::LogKind::Out,
        wire::LogKind::Err => report::LogKind::Err,
        wire::LogKind::Info => report::LogKind::Info,
        wire::LogKind::Success => report::LogKind::Success,
        wire::LogKind::Warn => report::LogKind::Warn,
        wire::LogKind::Error => report::LogKind::Error,
    }
}

fn outcome(outcome: wire::Outcome) -> report::Outcome {
    match outcome {
        wire::Outcome::Succeeded { release } => report::Outcome::Succeeded { release },
        wire::Outcome::Failed { error } => report::Outcome::Failed { error },
        wire::Outcome::RolledBack { error, to } => report::Outcome::RolledBack { error, to },
        wire::Outcome::RollbackFailed {
            error,
            rollback_error,
        } => report::Outcome::RollbackFailed {
            error,
            rollback_error,
        },
        wire::Outcome::Kept { release, error } => report::Outcome::Kept { release, error },
        wire::Outcome::Unchanged { live } => report::Outcome::Unchanged { live },
    }
}

/// A step's place in a list; a plan never has more than `u32` of them.
fn index(step: u32) -> usize {
    usize::try_from(step).unwrap_or(usize::MAX)
}

#[cfg(test)]
#[path = "convert_test.rs"]
mod tests;
