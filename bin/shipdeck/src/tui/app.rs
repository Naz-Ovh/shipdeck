//! Everything the screens show, and how keys and background results change it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::{ListState, TableState};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use shipdeck_engine::config::{Deployable, Discovery};
use shipdeck_engine::deploy::report::{
    LogKind, OpEvent, OpId, OpUpdate, Outcome, Reporter, StepState, Unit,
};
use shipdeck_engine::deploy::{self, ROLLBACK_STEPS};
use shipdeck_engine::remote::{Kind, Listing, Server, Status};
use shipdeck_engine::setup::{self, SetupCommand};

use super::clipboard;
use shipdeck_engine::ssh::{Pool, Question};

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
}

#[derive(Debug, Clone)]
pub enum Live {
    Unknown,
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

    fn apply(&mut self, event: OpEvent) {
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
            OpEvent::Finished(outcome) => self.finished = Some((outcome, Instant::now())),
        }
    }
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

pub enum Modal {
    ConfirmDeploy {
        deployable: usize,
    },
    ConfirmRollback {
        deployable: usize,
        target: String,
        kind: Kind,
        current: Option<String>,
    },
    ConfirmQuit,
    /// The one-time server setup command for a deployable.
    Setup {
        deployable: usize,
        setup: SetupCommand,
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
    pub broken: Vec<(PathBuf, String)>,
    pub live: Vec<Live>,
    pub list: ListState,
    pub screen: Screen,
    pub op: Option<Operation>,
    pub releases: Option<ReleasesView>,
    pub modals: VecDeque<Modal>,
    pub toast: Option<Toast>,
    pub tick: u64,
    pub quit: bool,
    /// The last setup command shown, with its deployable's name; printed on quit.
    pub last_setup: Option<(String, String)>,
    pub pool: Arc<Pool>,
    ops: UnboundedSender<OpUpdate>,
    fetched: UnboundedSender<Fetched>,
    next_op: OpId,
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
            broken: discovery.broken,
            live: vec![Live::Unknown; count],
            list,
            screen: Screen::Deployables,
            op: None,
            releases: None,
            modals: VecDeque::new(),
            toast: None,
            tick: 0,
            quit: false,
            last_setup: None,
            pool,
            ops,
            fetched,
            next_op: 1,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.list.selected().filter(|i| *i < self.deployables.len())
    }

    pub fn op_running(&self) -> bool {
        self.op.as_ref().is_some_and(Operation::running)
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
        if self
            .toast
            .as_ref()
            .is_some_and(|t| Instant::now() > t.until)
        {
            self.toast = None;
        }
    }

    // ── background work ────────────────────────────────────────────────

    pub fn refresh_all(&mut self) {
        for index in 0..self.deployables.len() {
            self.refresh_status(index);
        }
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
                Server::new(session, &deployable.config.remote)
                    .status()
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Status { index, result });
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
                Server::new(session, &deployable.config.remote).list().await
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
        }
    }

    pub fn on_op(&mut self, update: OpUpdate) {
        let Some(op) = self.op.as_mut().filter(|op| op.id == update.op) else {
            return;
        };
        let finished = matches!(update.event, OpEvent::Finished(_));
        op.apply(update.event);
        if finished {
            let index = op.deployable;
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
        self.op = Some(Operation::new(
            id,
            OpKind::Rollback {
                target: target.clone(),
            },
            index,
            &ROLLBACK_STEPS,
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

    /// Shows the one-time server setup command for one deployable. It is
    /// also printed when shipdeck quits, ready to copy.
    fn show_setup(&mut self, index: usize) {
        let setup = setup::command(&self.deployables[index], self.pool.login());
        self.last_setup = Some((
            self.deployables[index].config.name.clone(),
            setup.command.clone(),
        ));
        self.modals.push_back(Modal::Setup {
            deployable: index,
            setup,
        });
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
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
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
                        self.modals
                            .push_back(Modal::ConfirmDeploy { deployable: index });
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(index) = self.selected() {
                    self.open_releases(index);
                }
            }
            KeyCode::Char('s') => {
                self.refresh_all();
                self.notify("Refreshing live status", Tone::Info);
            }
            KeyCode::Char('u') => {
                if let Some(index) = self.selected() {
                    self.show_setup(index);
                }
            }
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
            kind: entry.kind,
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
            Modal::ConfirmDeploy { deployable } => {
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
            Modal::Setup { setup, .. } => {
                let command = setup.command.clone();
                if key.code == KeyCode::Char('c') {
                    match clipboard::copy(&command) {
                        Ok(()) => self.notify(
                            "Copied, if your terminal allows it; it is also printed when you quit",
                            Tone::Info,
                        ),
                        Err(err) => self.notify(format!("Could not copy: {err}"), Tone::Warn),
                    }
                } else if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q'))
                    || ctrl_c
                {
                    self.modals.pop_front();
                }
            }
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
