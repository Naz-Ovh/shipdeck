//! Everything the screens show, and how keys and background results change it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::{ListState, TableState};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use shipdeck_engine::bundle::files::{self, Source};
use shipdeck_engine::config::{Deployable, Discovery};
use shipdeck_engine::deploy;
use shipdeck_engine::deploy::report::{
    LogKind, OpEvent, OpId, OpUpdate, Outcome, Reporter, RollbackQuestion, StepState,
    UnchangedQuestion, Unit,
};
use shipdeck_engine::remote::{Archive, Control, Kind, Listing, Server, Status, log_command};
use shipdeck_engine::ssh::{Pool, Question};

use super::command::{self, CommandLine};

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
        /// What it is unpacked from first; `None` when it is open.
        from: Option<Archive>,
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

    // ── background work ────────────────────────────────────────────────

    /// Connects the deployable, or disconnects it when it is connected.
    fn toggle_connection(&mut self, index: usize) {
        match self.live[index] {
            Live::Loading => self.notify("Still connecting", Tone::Info),
            Live::Ready(_) => self.disconnect(index),
            Live::Disconnected | Live::Error(_) => self.refresh_status(index),
        }
    }

    /// Closes the session to the deployable's server. Every deployable on
    /// the same `user@host:port` shares it, so they all show disconnected.
    fn disconnect(&mut self, index: usize) {
        let endpoint = self.deployables[index].endpoint.clone();
        let in_use = self
            .op
            .as_ref()
            .filter(|op| op.running())
            .is_some_and(|op| self.deployables[op.deployable].endpoint == endpoint);
        if in_use {
            self.notify("An operation is using this connection", Tone::Warn);
            return;
        }
        for (deployable, live) in self.deployables.iter().zip(self.live.iter_mut()) {
            if deployable.endpoint == endpoint {
                *live = Live::Disconnected;
            }
        }
        self.notify(
            format!("Disconnected from {}", endpoint.label()),
            Tone::Info,
        );
        let pool = self.pool.clone();
        tokio::spawn(async move { pool.close(&endpoint).await });
    }

    /// Fetches the status again for every connected deployable.
    fn refresh_connected(&mut self) {
        let connected: Vec<usize> = (0..self.live.len())
            .filter(|index| matches!(self.live[*index], Live::Ready(_)))
            .collect();
        if connected.is_empty() {
            self.notify("Nothing is connected. Press c to connect", Tone::Info);
            return;
        }
        for index in connected {
            self.refresh_status(index);
        }
        self.notify("Refreshing live status", Tone::Info);
    }

    pub fn refresh_status(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        self.live[index] = Live::Loading;
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .status()
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Status { index, result });
        });
    }

    /// Stops, starts or restarts the deployable's service in the
    /// background.
    fn control(&mut self, index: usize, action: Control) {
        if self.controlling.contains(&index) {
            self.notify("Wait for the last command to finish", Tone::Warn);
            return;
        }
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        self.controlling.push(index);
        let name = deployable.config.service.name.clone();
        self.notify(format!("{} {name}…", doing(action)), Tone::Info);
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .control(action)
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Controlled {
                index,
                action,
                result,
            });
        });
    }

    fn controlled(&mut self, index: usize, action: Control, result: Result<(), String>) {
        self.controlling.retain(|i| *i != index);
        let Some(deployable) = self.deployables.get(index) else {
            return;
        };
        let name = deployable.config.service.name.clone();
        match result {
            Ok(()) => {
                self.notify(format!("{name}: {}", done(action)), Tone::Info);
                self.refresh_status(index);
            }
            Err(text) => self.modals.push_back(Modal::Notice {
                title: format!("Could not {} {name}", action.verb()),
                text,
            }),
        }
    }

    /// Opens the log view and follows the deployable's log until it closes.
    fn open_logs(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let view = self.next_logs;
        self.next_logs += 1;
        let (stop, stopped) = oneshot::channel::<()>();
        self.logs = Some(LogsView {
            id: view,
            deployable: index,
            command: log_command(&deployable.config.service),
            lines: VecDeque::new(),
            scroll: 0,
            state: LogsState::Connecting,
            _stop: stop,
        });
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                let server = Server::new(session, &deployable.config.service);
                let _ = tx.send(Fetched::LogsOpen { view });
                let line_tx = tx.clone();
                server
                    .follow_logs(
                        |line| {
                            let _ = line_tx.send(Fetched::LogLine {
                                view,
                                line: line.to_string(),
                            });
                        },
                        async {
                            // The view closed: its sender is gone.
                            let _ = stopped.await;
                        },
                    )
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::LogsEnded { view, result });
        });
    }

    fn open_releases(&mut self, index: usize) {
        let mut table = TableState::default();
        table.select(Some(0));
        self.releases = Some(ReleasesView {
            deployable: index,
            listing: Fetch::Loading,
            table,
        });
        self.screen = Screen::Releases;
        self.fetch_releases(index);
    }

    fn fetch_releases(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        if let Some(view) = self.releases.as_mut().filter(|v| v.deployable == index) {
            view.listing = Fetch::Loading;
        }
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .list()
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Releases { index, result });
        });
    }

    pub fn on_fetched(&mut self, fetched: Fetched) {
        match fetched {
            Fetched::Status { index, result } => {
                if let Some(slot) = self.live.get_mut(index) {
                    *slot = match result {
                        Ok(status) => Live::Ready(status),
                        Err(err) => Live::Error(err),
                    };
                }
            }
            Fetched::Releases { index, result } => {
                if let Some(view) = self.releases.as_mut().filter(|v| v.deployable == index) {
                    view.listing = match result {
                        Ok(listing) => {
                            let rows = listing.entries.len();
                            let selected = view.table.selected().unwrap_or(0);
                            view.table
                                .select((rows > 0).then(|| selected.min(rows - 1)));
                            Fetch::Ready(listing)
                        }
                        Err(err) => Fetch::Error(err),
                    };
                }
            }
            Fetched::Controlled {
                index,
                action,
                result,
            } => self.controlled(index, action, result),
            Fetched::LogsOpen { view } => {
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == view) {
                    logs.state = LogsState::Following;
                }
            }
            Fetched::LogLine { view, line } => {
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == view) {
                    logs.lines.push_back(line);
                    if logs.lines.len() > LOG_LIMIT {
                        logs.lines.pop_front();
                    }
                    // Keep a scrolled-up view still while new lines arrive.
                    if logs.scroll > 0 {
                        logs.scroll = (logs.scroll + 1).min(logs.lines.len());
                    }
                }
            }
            Fetched::LogsEnded { view, result } => {
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == view) {
                    logs.state = LogsState::Ended(match result {
                        Ok(Some(code)) => format!("the log ended (exit {code})"),
                        Ok(None) => "the log ended".to_string(),
                        Err(err) => err,
                    });
                }
            }
        }
    }

    pub fn on_op(&mut self, update: OpUpdate) {
        let Some(op) = self.op.as_mut().filter(|op| op.id == update.op) else {
            return;
        };
        let finished = matches!(update.event, OpEvent::Finished(_));
        let index = op.deployable;
        match op.apply(update.event) {
            Some(Asked::Rollback(question)) => {
                let deadline = Instant::now() + Duration::from_secs(question.seconds);
                // In front of everything else: the countdown is already running.
                self.modals.push_front(Modal::AskRollback {
                    op: update.op,
                    deployable: index,
                    question,
                    deadline,
                });
                self.screen = Screen::Operation;
            }
            Some(Asked::Unchanged(question)) => {
                // The deploy waits for it.
                self.modals.push_front(Modal::AskUnchanged {
                    op: update.op,
                    deployable: index,
                    question,
                });
                self.screen = Screen::Operation;
            }
            None => {}
        }
        if finished {
            let op_id = update.op;
            self.modals.retain(|modal| {
                !matches!(modal, Modal::AskRollback { op, .. } | Modal::AskUnchanged { op, .. } if *op == op_id)
            });
            self.refresh_status(index);
            if self
                .releases
                .as_ref()
                .is_some_and(|v| v.deployable == index)
            {
                self.fetch_releases(index);
            }
        }
    }

    pub fn on_question(&mut self, question: Question) {
        self.modals.push_back(match question {
            Question::Secret {
                title,
                prompt,
                reply,
            } => Modal::Secret {
                title,
                prompt,
                input: String::new(),
                reply: Some(reply),
            },
            Question::TrustHost {
                host,
                algorithm,
                fingerprint,
                reply,
            } => Modal::TrustHost {
                host,
                algorithm,
                fingerprint,
                reply: Some(reply),
            },
        });
    }

    fn start_deploy(&mut self, index: usize) {
        if self.op_running() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let deployable = self.deployables[index].clone();
        let id = self.next_op_id();
        let steps = deploy::deploy_step_names(&deployable.config);
        self.op = Some(Operation::new(id, OpKind::Deploy, index, &steps));
        self.screen = Screen::Operation;
        let reporter = Reporter::new(id, self.ops.clone());
        tokio::spawn(deploy::deploy(deployable, self.pool.clone(), reporter));
    }

    fn start_rollback(&mut self, index: usize, target: String) {
        if self.op_running() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let deployable = self.deployables[index].clone();
        let id = self.next_op_id();
        let steps = deploy::rollback_step_names(&deployable.config);
        self.op = Some(Operation::new(
            id,
            OpKind::Rollback {
                target: target.clone(),
            },
            index,
            &steps,
        ));
        self.screen = Screen::Operation;
        let reporter = Reporter::new(id, self.ops.clone());
        tokio::spawn(deploy::rollback(
            deployable,
            target,
            self.pool.clone(),
            reporter,
        ));
    }

    fn next_op_id(&mut self) -> OpId {
        let id = self.next_op;
        self.next_op += 1;
        id
    }

    // ── input ──────────────────────────────────────────────────────────

    pub fn on_event(&mut self, event: Event) {
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        if !self.modals.is_empty() {
            self.on_modal_key(key);
            return;
        }
        if self.command.is_some() {
            self.on_command_key(key);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return;
        }
        if self.logs.is_some() {
            self.on_logs_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => return self.request_quit(),
            KeyCode::Tab => return self.cycle_screen(1),
            KeyCode::BackTab => return self.cycle_screen(Screen::ALL.len() - 1),
            KeyCode::Char('1') => return self.screen = Screen::Deployables,
            KeyCode::Char('2') => return self.show_operation(),
            KeyCode::Char('3') => return self.show_releases(),
            _ => {}
        }
        match self.screen {
            Screen::Deployables => self.on_deployables_key(key),
            Screen::Operation => self.on_operation_key(key),
            Screen::Releases => self.on_releases_key(key),
        }
    }

    fn request_quit(&mut self) {
        if self.op_running() {
            self.modals.push_back(Modal::ConfirmQuit);
        } else {
            self.quit = true;
        }
    }

    fn cycle_screen(&mut self, by: usize) {
        let index = Screen::ALL
            .iter()
            .position(|s| *s == self.screen)
            .unwrap_or(0);
        match Screen::ALL[(index + by) % Screen::ALL.len()] {
            Screen::Deployables => self.screen = Screen::Deployables,
            Screen::Operation => self.show_operation(),
            Screen::Releases => self.show_releases(),
        }
    }

    fn show_operation(&mut self) {
        if self.op.is_some() {
            self.screen = Screen::Operation;
        } else {
            self.notify(
                "Nothing has run yet. Press ⏎ on a deployable to deploy it",
                Tone::Info,
            );
        }
    }

    fn show_releases(&mut self) {
        if self.releases.is_some() {
            self.screen = Screen::Releases;
        } else if let Some(index) = self.selected() {
            self.open_releases(index);
        }
    }

    fn on_deployables_key(&mut self, key: KeyEvent) {
        let count = self.deployables.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if count > 0 => {
                let i = self.selected().unwrap_or(0);
                self.list
                    .select(Some(if i == 0 { count - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') if count > 0 => {
                let i = self.selected().unwrap_or(0);
                self.list.select(Some((i + 1) % count));
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if let Some(index) = self.selected() {
                    if self.op_running() {
                        self.notify("An operation is already running", Tone::Warn);
                    } else {
                        let missing = files::walk_source(&self.deployables[index], Source::Deploy)
                            .err()
                            .map(|err| format!("{err:#}"));
                        self.modals.push_back(Modal::ConfirmDeploy {
                            deployable: index,
                            missing,
                        });
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(index) = self.selected() {
                    self.open_releases(index);
                }
            }
            KeyCode::Char('c') => {
                if let Some(index) = self.selected() {
                    self.toggle_connection(index);
                }
            }
            KeyCode::Char('s') => self.refresh_connected(),
            KeyCode::Char(':') if self.selected().is_some() => {
                self.command = Some(CommandLine::default());
            }
            _ => {}
        }
    }

    fn on_command_key(&mut self, key: KeyEvent) {
        let Some(line) = self.command.as_mut() else {
            return;
        };
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        match key.code {
            _ if ctrl_c => self.command = None,
            KeyCode::Esc => self.command = None,
            KeyCode::Enter => {
                let input = std::mem::take(&mut line.input);
                self.command = None;
                self.run_command(&input);
            }
            KeyCode::Tab => line.complete(),
            KeyCode::Backspace => {
                if !line.pop() {
                    self.command = None;
                }
            }
            KeyCode::Char(c) => line.push(c),
            _ => {}
        }
    }

    /// Runs what was typed after `:` for the selected deployable.
    fn run_command(&mut self, input: &str) {
        let Some(index) = self.selected() else {
            return;
        };
        if input.trim().is_empty() {
            return;
        }
        let command = match command::parse(input) {
            Ok(command) => command,
            Err(why) => return self.notify(why, Tone::Warn),
        };
        let Some(action) = command.control() else {
            return self.open_logs(index);
        };
        if self.op_running_on(index) {
            let slug = &self.deployables[index].slug;
            self.notify(
                format!("{slug} is being deployed; only :logs works until it ends"),
                Tone::Warn,
            );
            return;
        }
        if matches!(action, Control::Stop | Control::Restart) {
            self.modals.push_back(Modal::ConfirmControl {
                deployable: index,
                action,
            });
        } else {
            self.control(index, action);
        }
    }

    fn on_logs_key(&mut self, key: KeyEvent) {
        let Some(logs) = self.logs.as_mut() else {
            return;
        };
        let max = logs.lines.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.logs = None,
            KeyCode::Up | KeyCode::Char('k') => logs.scroll = (logs.scroll + 1).min(max),
            KeyCode::Down | KeyCode::Char('j') => logs.scroll = logs.scroll.saturating_sub(1),
            KeyCode::PageUp => logs.scroll = (logs.scroll + 20).min(max),
            KeyCode::PageDown => logs.scroll = logs.scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => logs.scroll = max,
            KeyCode::End | KeyCode::Char('G') => logs.scroll = 0,
            _ => {}
        }
    }

    fn on_operation_key(&mut self, key: KeyEvent) {
        let Some(op) = self.op.as_mut() else {
            return;
        };
        let max = op.logs.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => op.scroll = (op.scroll + 1).min(max),
            KeyCode::Down | KeyCode::Char('j') => op.scroll = op.scroll.saturating_sub(1),
            KeyCode::PageUp => op.scroll = (op.scroll + 20).min(max),
            KeyCode::PageDown => op.scroll = op.scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => op.scroll = max,
            KeyCode::End | KeyCode::Char('G') => op.scroll = 0,
            KeyCode::Char('r') => {
                let index = op.deployable;
                self.open_releases(index);
            }
            KeyCode::Esc | KeyCode::Backspace => self.screen = Screen::Deployables,
            _ => {}
        }
    }

    fn on_releases_key(&mut self, key: KeyEvent) {
        let Some(view) = self.releases.as_mut() else {
            return;
        };
        let rows = match &view.listing {
            Fetch::Ready(listing) => listing.entries.len(),
            _ => 0,
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if rows > 0 => {
                let i = view.table.selected().unwrap_or(0);
                view.table
                    .select(Some(if i == 0 { rows - 1 } else { i - 1 }));
            }
            KeyCode::Down | KeyCode::Char('j') if rows > 0 => {
                let i = view.table.selected().unwrap_or(0);
                view.table.select(Some((i + 1) % rows));
            }
            KeyCode::Char('r') => {
                let index = view.deployable;
                self.fetch_releases(index);
            }
            KeyCode::Enter => self.ask_rollback(),
            KeyCode::Esc | KeyCode::Backspace => self.screen = Screen::Deployables,
            _ => {}
        }
    }

    fn ask_rollback(&mut self) {
        let Some(view) = self.releases.as_ref() else {
            return;
        };
        let Fetch::Ready(listing) = &view.listing else {
            return;
        };
        let Some(entry) = view.table.selected().and_then(|i| listing.entries.get(i)) else {
            return;
        };
        if entry.current {
            self.notify("That release is already live", Tone::Info);
            return;
        }
        if entry.name.is_none() {
            self.notify(
                "shipdeck did not create that folder, so it will not switch to it",
                Tone::Warn,
            );
            return;
        }
        if self.op_running() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let modal = Modal::ConfirmRollback {
            deployable: view.deployable,
            target: entry.id.clone(),
            from: entry.backup.filter(|_| entry.kind == Kind::Compressed),
            current: listing.current.clone(),
        };
        self.modals.push_back(modal);
    }

    fn on_modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modals.front_mut() else {
            return;
        };
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        match modal {
            Modal::Secret { input, reply, .. } => match key.code {
                KeyCode::Enter => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(Some(std::mem::take(input)));
                    }
                    self.modals.pop_front();
                }
                KeyCode::Esc => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(None);
                    }
                    self.modals.pop_front();
                }
                _ if ctrl_c => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(None);
                    }
                    self.modals.pop_front();
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear()
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Modal::TrustHost { reply, .. } => {
                let answer = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
                    _ if ctrl_c => Some(false),
                    _ => None,
                };
                if let Some(answer) = answer {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(answer);
                    }
                    self.modals.pop_front();
                }
            }
            Modal::ConfirmDeploy { deployable, .. } => {
                let index = *deployable;
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.modals.pop_front();
                        self.start_deploy(index);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.modals.pop_front();
                    }
                    _ if ctrl_c => {
                        self.modals.pop_front();
                    }
                    _ => {}
                }
            }
            Modal::ConfirmRollback {
                deployable, target, ..
            } => {
                let (index, target) = (*deployable, target.clone());
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.modals.pop_front();
                        self.start_rollback(index, target);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.modals.pop_front();
                    }
                    _ if ctrl_c => {
                        self.modals.pop_front();
                    }
                    _ => {}
                }
            }
            Modal::AskRollback { .. } => {
                let answer = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => true,
                    KeyCode::Char('n') | KeyCode::Char('N') => false,
                    _ => return,
                };
                if let Some(Modal::AskRollback { question, .. }) = self.modals.pop_front() {
                    // The deploy may have stopped waiting a moment ago.
                    let _ = question.reply.send(answer);
                }
            }
            Modal::AskUnchanged { .. } => {
                let answer = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => true,
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                        false
                    }
                    _ if ctrl_c => false,
                    _ => return,
                };
                if let Some(Modal::AskUnchanged { question, .. }) = self.modals.pop_front() {
                    let _ = question.reply.send(answer);
                }
            }
            Modal::ConfirmControl { deployable, action } => {
                let (index, action) = (*deployable, *action);
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        self.modals.pop_front();
                        if self.op_running_on(index) {
                            self.notify("A deploy started meanwhile; not now", Tone::Warn);
                        } else {
                            self.control(index, action);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        self.modals.pop_front();
                    }
                    _ if ctrl_c => {
                        self.modals.pop_front();
                    }
                    _ => {}
                }
            }
            Modal::Notice { .. } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                    self.modals.pop_front();
                }
                _ if ctrl_c => {
                    self.modals.pop_front();
                }
                _ => {}
            },
            Modal::ConfirmQuit => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.quit = true,
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.modals.pop_front();
                }
                _ if ctrl_c => self.quit = true,
                _ => {}
            },
        }
    }
}

/// `Stopping`, `Starting`, `Restarting`.
fn doing(action: Control) -> &'static str {
    match action {
        Control::Stop => "Stopping",
        Control::Start => "Starting",
        Control::Restart => "Restarting",
    }
}

/// `stopped`, `started`, `restarted`.
fn done(action: Control) -> &'static str {
    match action {
        Control::Stop => "stopped",
        Control::Start => "started",
        Control::Restart => "restarted",
    }
}
