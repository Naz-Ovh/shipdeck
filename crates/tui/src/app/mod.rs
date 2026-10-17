//! Everything the screens show. Starting work, results from the background
//! and keys each have their own file here.

mod actions;
mod details;
mod events;
mod keys;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::widgets::{ListState, TableState};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use shipdeck_engine::config::{Deployable, Discovery};
use shipdeck_engine::deploy::report::{
    LogKind, OpEvent, OpId, OpUpdate, Outcome, RollbackQuestion, StepState, UnchangedQuestion, Unit,
};
use shipdeck_engine::remote::{
    Control, FileInfo, FileText, Listing, ReleaseContents, ReleaseEntry, Source, Status,
};
use shipdeck_engine::ssh::Pool;

use crate::command::CommandLine;

/// Log lines kept per operation; older ones scroll away.
const LOG_LIMIT: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Deployables,
    Operation,
    Releases,
}

impl Screen {
    pub const ALL: [Screen; 3] = [Screen::Deployables, Screen::Operation, Screen::Releases];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Deployables => "Deployables",
            Screen::Operation => "Operation",
            Screen::Releases => "Releases",
        }
    }
}

/// A result from a background fetch.
#[derive(Debug)]
pub enum Fetched {
    Status {
        index: usize,
        result: Result<Status, String>,
    },
    Releases {
        index: usize,
        result: Result<Listing, String>,
    },
    /// What release `id` holds, for the details popup.
    Contents {
        index: usize,
        id: String,
        source: Source,
        result: Result<ReleaseContents, String>,
    },
    /// The start of one file of release `id`.
    FileText {
        index: usize,
        id: String,
        path: String,
        result: Result<FileText, String>,
    },
    /// A `:stop`, `:start` or `:restart` is done.
    Controlled {
        index: usize,
        action: Control,
        result: Result<(), String>,
    },
    /// The log view `view` is connected and following.
    LogsOpen {
        view: u64,
    },
    LogLine {
        view: u64,
        line: String,
    },
    /// The log ended: its exit code when it ended on its own.
    LogsEnded {
        view: u64,
        result: Result<Option<u32>, String>,
    },
}

/// One deployable's connection and what it last reported. Nothing connects
/// until asked: `c` on the deployable, or a deploy, rollback or release list.
#[derive(Debug, Clone)]
pub enum Live {
    Disconnected,
    Loading,
    Ready(Status),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub unit: Unit,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct StepView {
    pub name: &'static str,
    pub state: StepState,
    pub detail: Option<String>,
    pub progress: Option<Progress>,
    pub started: Option<Instant>,
    pub ended: Option<Instant>,
}

impl StepView {
    pub fn elapsed(&self) -> Option<Duration> {
        let started = self.started?;
        Some(
            self.ended
                .unwrap_or_else(Instant::now)
                .duration_since(started),
        )
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
    pub at: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    Deploy,
    Rollback { target: String },
}

pub struct Operation {
    pub id: OpId,
    pub kind: OpKind,
    pub deployable: usize,
    pub release: Option<String>,
    pub steps: Vec<StepView>,
    pub logs: VecDeque<LogLine>,
    /// Lines up from the bottom; 0 follows new output.
    pub scroll: usize,
    pub started: Instant,
    pub finished: Option<(Outcome, Instant)>,
}

impl Operation {
    pub(super) fn new(id: OpId, kind: OpKind, deployable: usize, steps: &[&'static str]) -> Self {
        Self {
            id,
            kind,
            deployable,
            release: None,
            steps: steps
                .iter()
                .map(|name| StepView {
                    name,
                    state: StepState::Pending,
                    detail: None,
                    progress: None,
                    started: None,
                    ended: None,
                })
                .collect(),
            logs: VecDeque::new(),
            scroll: 0,
            started: Instant::now(),
            finished: None,
        }
    }

    pub fn running(&self) -> bool {
        self.finished.is_none()
    }

    pub fn elapsed(&self) -> Duration {
        self.finished
            .as_ref()
            .map(|(_, at)| *at)
            .unwrap_or_else(Instant::now)
            .duration_since(self.started)
    }

    /// The step to show in the progress panel: the running one, else the
    /// one that failed, else the last one that reported progress.
    pub fn focus_step(&self) -> Option<&StepView> {
        let find = |state| self.steps.iter().find(|s| s.state == state);
        find(StepState::Running)
            .or_else(|| find(StepState::Failed))
            .or_else(|| self.steps.iter().rev().find(|s| s.progress.is_some()))
    }

    pub fn completed_steps(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| !matches!(s.state, StepState::Pending | StepState::Running))
            .count()
    }

    /// Applies one event; a question is handed back for a dialog.
    fn apply(&mut self, event: OpEvent) -> Option<Asked> {
        match event {
            OpEvent::Step {
                step,
                state,
                detail,
            } => {
                if let Some(view) = self.steps.get_mut(step) {
                    let now = Instant::now();
                    if state == StepState::Running {
                        view.started = Some(now);
                        view.ended = None;
                    } else if view.started.is_some() && view.ended.is_none() {
                        view.ended = Some(now);
                    }
                    view.state = state;
                    // A finished step's bar ends full, not where the last update left it.
                    if state == StepState::Done
                        && let Some(progress) = view.progress.as_mut()
                        && progress.total > 0
                    {
                        progress.done = progress.total;
                    }
                    if detail.is_some() || state == StepState::Running {
                        view.detail = detail;
                    }
                }
            }
            OpEvent::Progress {
                step,
                done,
                total,
                unit,
                label,
            } => {
                if let Some(view) = self.steps.get_mut(step) {
                    view.progress = Some(Progress {
                        done,
                        total,
                        unit,
                        label,
                    });
                }
            }
            OpEvent::Log { kind, text } => {
                self.logs.push_back(LogLine {
                    kind,
                    text,
                    at: self.started.elapsed(),
                });
                if self.logs.len() > LOG_LIMIT {
                    self.logs.pop_front();
                }
                // Keep a scrolled-up view still while new lines arrive.
                if self.scroll > 0 {
                    self.scroll = (self.scroll + 1).min(self.logs.len());
                }
            }
            OpEvent::Release(id) => self.release = Some(id),
            OpEvent::AskRollback(question) => return Some(Asked::Rollback(question)),
            OpEvent::AskUnchanged(question) => return Some(Asked::Unchanged(question)),
            OpEvent::Finished(outcome) => self.finished = Some((outcome, Instant::now())),
        }
        None
    }
}

/// A question a running operation asks.
enum Asked {
    Rollback(RollbackQuestion),
    Unchanged(UnchangedQuestion),
}

#[derive(Debug, Clone)]
pub enum Fetch<T> {
    Loading,
    Ready(T),
    Error(String),
}

pub struct ReleasesView {
    pub deployable: usize,
    pub listing: Fetch<Listing>,
    pub table: TableState,
    /// The release shown in the details popup (`D`).
    pub details: Option<ReleaseDetails>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailsTab {
    Details,
    Files,
}

/// One release looked at closely: its `release.json` and where it lives,
/// and every file it holds, from its folder or from inside its backup.
pub struct ReleaseDetails {
    pub entry: ReleaseEntry,
    pub tab: DetailsTab,
    pub source: Source,
    pub contents: Fetch<ReleaseContents>,
    /// The file picked in the Files tab.
    pub selected: usize,
    /// The file being read, over the list.
    pub file: Option<OpenFile>,
}

pub struct OpenFile {
    pub info: FileInfo,
    pub text: Fetch<FileText>,
    /// The first line shown.
    pub scroll: usize,
}

/// Where the log view is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogsState {
    Connecting,
    Following,
    /// Why it stopped.
    Ended(String),
}

/// A service's log, followed live over the whole screen.
pub struct LogsView {
    pub id: u64,
    pub deployable: usize,
    /// What follows it on the server.
    pub command: String,
    pub lines: VecDeque<String>,
    /// Lines up from the bottom; 0 follows new output.
    pub scroll: usize,
    pub state: LogsState,
    /// Dropping it, as closing the view does, stops the log on the server.
    _stop: oneshot::Sender<()>,
}

pub enum Modal {
    ConfirmDeploy {
        deployable: usize,
        /// Why files from the deploy folder cannot be shipped yet, if so.
        missing: Option<String>,
    },
    ConfirmRollback {
        deployable: usize,
        target: String,
        /// Unpacked from its backup and set up first, as it is not open.
        from_backup: bool,
        current: Option<String>,
    },
    ConfirmQuit,
    /// `:stop` or `:restart` asks first.
    ConfirmControl {
        deployable: usize,
        action: Control,
    },
    /// Something that needs reading, such as why a command failed.
    Notice {
        title: String,
        text: String,
    },
    /// A release failed after the switch: roll back, or keep it? With no
    /// answer by `deadline` the deploy rolls back on its own.
    AskRollback {
        op: OpId,
        deployable: usize,
        question: RollbackQuestion,
        deadline: Instant,
    },
    /// The release holds what the live one does: deploy a duplicate? The
    /// deploy waits; no means it stops.
    AskUnchanged {
        op: OpId,
        deployable: usize,
        question: UnchangedQuestion,
    },
    Secret {
        title: String,
        prompt: String,
        input: String,
        reply: Option<oneshot::Sender<Option<String>>>,
    },
    TrustHost {
        host: String,
        algorithm: String,
        fingerprint: String,
        reply: Option<oneshot::Sender<bool>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warn,
}

pub struct Toast {
    pub text: String,
    pub tone: Tone,
    pub until: Instant,
}

pub struct App {
    pub root: PathBuf,
    pub deployables: Vec<Deployable>,
    pub live: Vec<Live>,
    pub list: ListState,
    pub screen: Screen,
    pub op: Option<Operation>,
    pub releases: Option<ReleasesView>,
    /// The `:` line, while it is open.
    pub command: Option<CommandLine>,
    /// The log view, while it is open; it covers every screen.
    pub logs: Option<LogsView>,
    /// Deployables a `:stop`, `:start` or `:restart` is running for.
    pub controlling: Vec<usize>,
    pub modals: VecDeque<Modal>,
    pub toast: Option<Toast>,
    pub tick: u64,
    pub quit: bool,
    pub pool: Arc<Pool>,
    ops: UnboundedSender<OpUpdate>,
    fetched: UnboundedSender<Fetched>,
    next_op: OpId,
    next_logs: u64,
}

impl App {
    pub fn new(
        root: PathBuf,
        discovery: Discovery,
        pool: Arc<Pool>,
        ops: UnboundedSender<OpUpdate>,
        fetched: UnboundedSender<Fetched>,
    ) -> Self {
        let count = discovery.deployables.len();
        let mut list = ListState::default();
        if count > 0 {
            list.select(Some(0));
        }
        Self {
            root,
            deployables: discovery.deployables,
            live: vec![Live::Disconnected; count],
            list,
            screen: Screen::Deployables,
            op: None,
            releases: None,
            command: None,
            logs: None,
            controlling: Vec::new(),
            modals: VecDeque::new(),
            toast: None,
            tick: 0,
            quit: false,
            pool,
            ops,
            fetched,
            next_op: 1,
            next_logs: 1,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.list.selected().filter(|i| *i < self.deployables.len())
    }

    pub fn op_running(&self) -> bool {
        self.op.as_ref().is_some_and(Operation::running)
    }

    /// Whether a deploy or rollback of `index` is running.
    fn op_running_on(&self, index: usize) -> bool {
        self.op
            .as_ref()
            .is_some_and(|op| op.running() && op.deployable == index)
    }

    fn notify(&mut self, text: impl Into<String>, tone: Tone) {
        self.toast = Some(Toast {
            text: text.into(),
            tone,
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        let now = Instant::now();
        if self.toast.as_ref().is_some_and(|t| now > t.until) {
            self.toast = None;
        }
        // The deploy has stopped waiting by now and rolls back on its own.
        self.modals.retain(
            |modal| !matches!(modal, Modal::AskRollback { deadline, .. } if now >= *deadline),
        );
    }
}
