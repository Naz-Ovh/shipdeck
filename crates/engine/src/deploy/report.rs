//! Reports from a running deploy or rollback to the screen: step changes,
//! progress, log lines and the final outcome.

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

pub type OpId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Running,
    Done,
    Warned,
    Failed,
    Skipped,
}

/// What a progress count measures, so the screen can format it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Items,
    Seconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// A step starts.
    Step,
    /// A command is about to run, locally or on the server.
    Command,
    Out,
    Err,
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Succeeded {
        release: String,
    },
    /// Stopped before the live version was touched.
    Failed {
        error: String,
    },
    /// The new version failed and the previous one is live again.
    RolledBack {
        error: String,
        to: String,
    },
    /// The new version failed and putting the old one back failed too.
    RollbackFailed {
        error: String,
        rollback_error: String,
    },
    /// The new version failed after the switch, and you chose to keep it.
    Kept {
        release: String,
        error: String,
    },
    /// The release held exactly what `live` does, and you chose not to
    /// deploy a duplicate; nothing on the server changed.
    Unchanged {
        live: String,
    },
}

/// The deploy asks whether to roll back. No answer within `seconds` (or
/// nobody to answer) means yes.
#[derive(Debug)]
pub struct RollbackQuestion {
    /// The release that failed.
    pub release: String,
    /// The release that was live before it.
    pub previous: String,
    /// The step that failed, such as `Health`.
    pub step: &'static str,
    pub error: String,
    pub seconds: u64,
    /// `true` rolls back; `false` keeps the new release.
    pub reply: oneshot::Sender<bool>,
}

/// The deploy asks whether to go on with a release that holds exactly
/// what the live one does. No answer (nobody to answer) means no.
#[derive(Debug)]
pub struct UnchangedQuestion {
    /// The release the deploy would make.
    pub release: String,
    /// The live release with the same content.
    pub live: String,
    /// `true` deploys it anyway, as a duplicate of `live`.
    pub reply: oneshot::Sender<bool>,
}

#[derive(Debug)]
pub enum OpEvent {
    Step {
        step: usize,
        state: StepState,
        detail: Option<String>,
    },
    Progress {
        step: usize,
        done: u64,
        total: u64,
        unit: Unit,
        label: String,
    },
    Log {
        kind: LogKind,
        text: String,
    },
    /// The id of the release this operation works on, once it is known.
    Release(String),
    AskRollback(RollbackQuestion),
    AskUnchanged(UnchangedQuestion),
    Finished(Outcome),
}

#[derive(Debug)]
pub struct OpUpdate {
    pub op: OpId,
    pub event: OpEvent,
}

/// Cheap to clone; every clone reports into the same operation.
#[derive(Debug, Clone)]
pub struct Reporter {
    op: OpId,
    tx: UnboundedSender<OpUpdate>,
}

impl Reporter {
    pub fn new(op: OpId, tx: UnboundedSender<OpUpdate>) -> Self {
        Self { op, tx }
    }

    fn send(&self, event: OpEvent) {
        // The screen may already be gone while the app quits; nothing to do then.
        let _ = self.tx.send(OpUpdate { op: self.op, event });
    }

    pub fn start(&self, step: usize, name: &str) {
        self.send(OpEvent::Step {
            step,
            state: StepState::Running,
            detail: None,
        });
        self.log(LogKind::Step, name);
    }

    pub fn finish(&self, step: usize, detail: impl Into<String>) {
        self.set(step, StepState::Done, detail);
    }

    pub fn warn(&self, step: usize, detail: impl Into<String>) {
        self.set(step, StepState::Warned, detail);
    }

    pub fn fail(&self, step: usize, detail: impl Into<String>) {
        self.set(step, StepState::Failed, detail);
    }

    pub fn skip(&self, step: usize, detail: impl Into<String>) {
        self.set(step, StepState::Skipped, detail);
    }

    fn set(&self, step: usize, state: StepState, detail: impl Into<String>) {
        let detail = detail.into();
        self.send(OpEvent::Step {
            step,
            state,
            detail: (!detail.is_empty()).then_some(detail),
        });
    }

    /// `total == 0` means the end is not known yet.
    pub fn progress(
        &self,
        step: usize,
        done: u64,
        total: u64,
        unit: Unit,
        label: impl Into<String>,
    ) {
        self.send(OpEvent::Progress {
            step,
            done,
            total,
            unit,
            label: label.into(),
        });
    }

    pub fn log(&self, kind: LogKind, text: impl Into<String>) {
        self.send(OpEvent::Log {
            kind,
            text: text.into(),
        });
    }

    pub fn release(&self, id: &str) {
        self.send(OpEvent::Release(id.to_string()));
    }

    /// Asks whether to roll back; the answer arrives on the receiver.
    pub fn ask_rollback(
        &self,
        release: &str,
        previous: &str,
        step: &'static str,
        error: &str,
        seconds: u64,
    ) -> oneshot::Receiver<bool> {
        let (reply, answer) = oneshot::channel();
        self.send(OpEvent::AskRollback(RollbackQuestion {
            release: release.to_string(),
            previous: previous.to_string(),
            step,
            error: error.to_string(),
            seconds,
            reply,
        }));
        answer
    }

    /// Asks whether to deploy a release the live one already holds; the
    /// answer arrives on the receiver.
    pub fn ask_unchanged(&self, release: &str, live: &str) -> oneshot::Receiver<bool> {
        let (reply, answer) = oneshot::channel();
        self.send(OpEvent::AskUnchanged(UnchangedQuestion {
            release: release.to_string(),
            live: live.to_string(),
            reply,
        }));
        answer
    }

    pub fn done(&self, outcome: Outcome) {
        self.send(OpEvent::Finished(outcome));
    }
}
