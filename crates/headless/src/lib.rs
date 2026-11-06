//! Deploy, rollback and the release list without the TUI, for scripts and
//! CI: progress goes to stderr as plain lines, nothing waits for a person,
//! and the exit code says how it ended.
//!
//! The work is the engine's, in a child process; this crate only asks and
//! prints.

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use chrono::DateTime;

use shipdeck_protocol::connection::{Connection, Error, Handle, Incoming};
use shipdeck_protocol::message::ask::{Ask, SecretAnswer};
use shipdeck_protocol::message::common::{
    DeployableInfo, LogKind, OnFailure, Outcome, ReleaseKind, StepState, Unit,
};
use shipdeck_protocol::message::event::{Event, OpProgress, OpRelease, OpStep};
use shipdeck_protocol::message::method;
use shipdeck_protocol::message::request::{
    DeployStart, OperationStarted, ReleasesList, ReleasesListResult, RollbackStart,
};
use shipdeck_protocol::wire::Id;

use shipdeck_engine::human;

/// Ctrl-C pressed twice: the run was stopped where it was.
const INTERRUPTED: u8 = 130;

/// How often a long step reports how far it got.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// What the flags of this run say, and so how to answer the engine's
/// questions.
#[derive(Debug, Clone, Copy)]
pub struct Choices {
    /// Trust a server's new host key without asking.
    pub trust_new_host: bool,
    /// Deploy a release the live one already holds.
    pub allow_unchanged: bool,
    /// What a failure after the switch does; overrides `pipeline.failed`.
    pub on_failure: OnFailure,
}

/// A run against the engine: one command, one folder, no questions.
pub struct Headless {
    connection: Connection,
    paint: Paint,
    choices: Choices,
}

impl Headless {
    #[must_use]
    pub fn new(connection: Connection, choices: Choices) -> Self {
        Self {
            connection,
            paint: Paint::stderr(),
            choices,
        }
    }

    /// Deploys each in turn; the first that does not succeed stops the rest.
    pub async fn deploy(&mut self, projects: &[DeployableInfo], names: &[String]) -> ExitCode {
        let mut chosen: Vec<DeployableInfo> = Vec::new();
        for name in names {
            match find(projects, name) {
                Ok(found) if chosen.iter().any(|p| p.slug == found.slug) => {}
                Ok(found) => chosen.push(found.clone()),
                Err(err) => return self.fail(&err),
            }
        }
        for (index, project) in chosen.iter().enumerate() {
            self.heading("deploy", project);
            let asked = DeployStart {
                deployable: project.slug.clone(),
                on_failure: Some(self.choices.on_failure),
            };
            let started = match self
                .connection
                .request::<_, OperationStarted>(method::DEPLOY_START, &asked)
                .await
            {
                Ok(started) => started,
                Err(err) => return self.fail(&reason(&err)),
            };
            let code = self.watch(project, &started.steps).await;
            if code != 0 {
                let rest: Vec<&str> = chosen[index + 1..]
                    .iter()
                    .map(|p| p.slug.as_str())
                    .collect();
                if !rest.is_empty() && code != INTERRUPTED {
                    eprintln!(
                        "{}",
                        self.paint
                            .warn(&format!("▲ not deployed: {}", rest.join(", ")))
                    );
                }
                return ExitCode::from(code);
            }
        }
        ExitCode::SUCCESS
    }

    /// Puts one release back.
    pub async fn rollback(
        &mut self,
        projects: &[DeployableInfo],
        name: &str,
        to: String,
    ) -> ExitCode {
        let project = match find(projects, name) {
            Ok(found) => found.clone(),
            Err(err) => return self.fail(&err),
        };
        self.heading(&format!("rollback to {to}"), &project);
        // A rollback that fails puts back what was live; nothing to keep. It
        // never asks about an unchanged release.
        let asked = RollbackStart {
            deployable: project.slug.clone(),
            release: to,
        };
        let started = match self
            .connection
            .request::<_, OperationStarted>(method::ROLLBACK_START, &asked)
            .await
        {
            Ok(started) => started,
            Err(err) => return self.fail(&reason(&err)),
        };
        ExitCode::from(self.watch(&project, &started.steps).await)
    }

    /// Prints the releases on the server, newest first, to stdout.
    pub async fn releases(&mut self, projects: &[DeployableInfo], name: &str) -> ExitCode {
        let project = match find(projects, name) {
            Ok(found) => found.clone(),
            Err(err) => return self.fail(&err),
        };
        let asked = ReleasesList {
            deployable: project.slug.clone(),
        };
        let listing = self
            .connection
            .request::<_, ReleasesListResult>(method::RELEASES_LIST, &asked)
            .await;
        match listing {
            Ok(listing) if listing.entries.is_empty() => {
                eprintln!("no releases on the server yet");
                ExitCode::SUCCESS
            }
            Ok(listing) => {
                print!("{}", table(&listing));
                ExitCode::SUCCESS
            }
            Err(err) => self.fail(&reason(&err)),
        }
    }

    /// Says goodbye, so the engine stops taking work.
    pub async fn close(&self) {
        let _ = self.connection.shutdown().await;
    }

    /// Prints one operation's events until it ends; returns its exit code.
    async fn watch(&mut self, project: &DeployableInfo, steps: &[String]) -> u8 {
        let paint = self.paint;
        let choices = self.choices;
        let handle = self.connection.handle();
        let mut lines = Lines::new(steps, paint, Instant::now());
        let started = Instant::now();
        let mut interrupted = false;
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        loop {
            tokio::select! {
                incoming = self.connection.recv() => match incoming {
                    Some(Incoming::Event(Event::Finished(finished))) => {
                        eprintln!(
                            "{}",
                            summary(paint, &project.name, &finished.outcome, started.elapsed())
                        );
                        return exit_code(&finished.outcome);
                    }
                    Some(Incoming::Event(event)) => {
                        if let Some(line) = lines.line(event, Instant::now()) {
                            eprintln!("{line}");
                        }
                    }
                    Some(Incoming::Ask { id, ask }) => {
                        answer(&handle, paint, choices, id, ask);
                    }
                    // The engine asks nothing this version does not know, and
                    // sends no notification these commands read.
                    Some(Incoming::Other { .. } | Incoming::Malformed { .. }) => {}
                    None => {
                        eprintln!("{}", paint.error("✘ the operation stopped without a result"));
                        return 1;
                    }
                },
                _ = &mut ctrl_c => {
                    if interrupted {
                        eprintln!("{}", paint.error("✘ stopped"));
                        return INTERRUPTED;
                    }
                    interrupted = true;
                    eprintln!(
                        "{}",
                        paint.warn(
                            "▲ stopping now can leave the server between two releases; \
                             press Ctrl-C again to stop anyway"
                        )
                    );
                    ctrl_c.set(tokio::signal::ctrl_c());
                }
            }
        }
    }

    fn heading(&self, what: &str, project: &DeployableInfo) {
        eprintln!(
            "{}",
            self.paint.step(&format!(
                "━━ {} · {what} → {}:{}",
                project.name,
                label(project),
                project.dir
            ))
        );
    }

    fn fail(&self, error: &str) -> ExitCode {
        eprintln!("{}", self.paint.error(&format!("✘ {error}")));
        ExitCode::FAILURE
    }
}

/// What the engine said, without the protocol's own words around it.
fn reason(err: &Error) -> String {
    match err {
        Error::Rpc(object) => object.message.clone(),
        other => other.to_string(),
    }
}

/// `deploy@prod`, or `deploy@prod:2222` on another port: what the heading
/// shows before the server's folder.
fn label(project: &DeployableInfo) -> String {
    let user = project.user.as_deref().unwrap_or("");
    if project.port == 22 {
        format!("{user}@{}", project.host)
    } else {
        format!("{user}@{}:{}", project.host, project.port)
    }
}

/// Nobody is there to type: a new host key is trusted only with
/// `--trust-new-host`, a password or passphrase is never asked, and the two
/// choice questions are answered from the flags.
fn answer(handle: &Handle, paint: Paint, choices: Choices, id: Id, ask: Ask) {
    match ask {
        Ask::TrustHost(question) => {
            if choices.trust_new_host {
                eprintln!(
                    "{}",
                    paint.warn(&format!(
                        "▲ trusting the new host key of {} ({} {}) and saving it to \
                         ~/.ssh/known_hosts",
                        question.host, question.algorithm, question.fingerprint
                    ))
                );
            } else {
                eprintln!(
                    "{}",
                    paint.error(&format!(
                        "✘ {} is not in ~/.ssh/known_hosts; its key is {} {}. If that is right, \
                         run again with --trust-new-host",
                        question.host, question.algorithm, question.fingerprint
                    ))
                );
            }
            let _ = handle.answer(id, &choices.trust_new_host);
        }
        Ask::Secret(question) => {
            eprintln!(
                "{}",
                paint.error(&format!(
                    "✘ {}: nothing can be asked here. Set SHIPDECK_KEY_PASSPHRASE, use a key \
                     without a passphrase, or set SHIPDECK_PASSWORD",
                    question.prompt
                ))
            );
            let _ = handle.answer(id, &SecretAnswer { secret: None });
        }
        Ask::Unchanged(_) => {
            let _ = handle.answer(id, &choices.allow_unchanged);
        }
        Ask::Rollback(_) => {
            let _ = handle.answer(id, &(choices.on_failure == OnFailure::Rollback));
        }
    }
}

/// The exit code for each way an operation ends. 2 is left to the flag
/// parser's usage errors. Scripts depend on these: add, never renumber.
#[must_use]
pub fn exit_code(outcome: &Outcome) -> u8 {
    match outcome {
        Outcome::Succeeded { .. } => 0,
        Outcome::Failed { .. } => 1,
        Outcome::RolledBack { .. } => 3,
        Outcome::Kept { .. } => 4,
        Outcome::RollbackFailed { .. } => 5,
        Outcome::Unchanged { .. } => 6,
    }
}

/// The project called `name`: its folder name, else its `name` field.
///
/// # Errors
/// Returns the list of names when none or more than one matches.
pub fn find<'a>(projects: &'a [DeployableInfo], name: &str) -> Result<&'a DeployableInfo, String> {
    if let Some(found) = projects.iter().find(|p| p.slug == name) {
        return Ok(found);
    }
    let named: Vec<&DeployableInfo> = projects.iter().filter(|p| p.name == name).collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => {
            let known: Vec<&str> = projects.iter().map(|p| p.slug.as_str()).collect();
            Err(if known.is_empty() {
                format!("no deployable named {name}: the deploy root has none")
            } else {
                format!("no deployable named {name}; there is {}", known.join(", "))
            })
        }
        many => {
            let folders: Vec<&str> = many.iter().map(|p| p.slug.as_str()).collect();
            Err(format!(
                "{} deployables are named {name}; use a folder name: {}",
                many.len(),
                folders.join(", ")
            ))
        }
    }
}

/// The last line of an operation: what is live now. The error itself is
/// already in the log right above it.
#[must_use]
pub fn summary(paint: Paint, name: &str, outcome: &Outcome, took: Duration) -> String {
    match outcome {
        Outcome::Succeeded { release } => paint.success(&format!(
            "✔ {name}: {release} is live ({})",
            human::duration(took)
        )),
        Outcome::Failed { .. } => paint.error(&format!(
            "✘ {name}: failed; the live release was not touched"
        )),
        Outcome::RolledBack { to, .. } => paint.warn(&format!(
            "▲ {name}: the new release failed; {to} is live again"
        )),
        Outcome::Kept { release, .. } => {
            paint.warn(&format!("▲ {name}: {release} failed and stays live"))
        }
        Outcome::RollbackFailed { .. } => paint.error(&format!(
            "✘ {name}: the new release failed and putting the old one back failed too; \
             check the server now"
        )),
        Outcome::Unchanged { live } => paint.warn(&format!(
            "▲ {name}: nothing changed since {live}, so nothing was deployed; \
             --allow-unchanged deploys it anyway"
        )),
    }
}

/// Turns operation events into lines: logs as they come, each step once it
/// ends, and progress every few seconds while a long step runs.
pub struct Lines {
    steps: Vec<String>,
    paint: Paint,
    /// After a failure the remaining steps are all skipped; no need to list them.
    failed: bool,
    /// When progress was last printed, or the running step started.
    progress_at: Instant,
}

impl Lines {
    #[must_use]
    pub fn new(steps: &[String], paint: Paint, now: Instant) -> Self {
        Self {
            steps: steps.to_vec(),
            paint,
            failed: false,
            progress_at: now,
        }
    }

    pub fn line(&mut self, event: Event, now: Instant) -> Option<String> {
        match event {
            Event::Log(log) => Some(self.log(log.kind, &log.text)),
            Event::Step(step) => self.step(&step, now),
            Event::Progress(progress) => self.progress(&progress, now),
            Event::Release(OpRelease { release, .. }) => {
                Some(self.paint.info(&format!("ℹ release {release}")))
            }
            Event::Finished(_) | Event::LogsLine(_) | Event::LogsEnded(_) => None,
        }
    }

    fn log(&self, kind: LogKind, text: &str) -> String {
        let paint = self.paint;
        match kind {
            LogKind::Step => paint.step(&format!("━━ {text}")),
            LogKind::Command => paint.command(&format!("› {text}")),
            LogKind::Out | LogKind::Err => format!("  {text}"),
            LogKind::Info => paint.info(&format!("ℹ {text}")),
            LogKind::Success => paint.success(&format!("✔ {text}")),
            LogKind::Warn => paint.warn(&format!("▲ {text}")),
            LogKind::Error => paint.error(&format!("✘ {text}")),
        }
    }

    fn step(&mut self, step: &OpStep, now: Instant) -> Option<String> {
        let paint = self.paint;
        let place = usize::try_from(step.step).unwrap_or(usize::MAX);
        let name = self.steps.get(place).map_or("step", String::as_str);
        let detail = step
            .detail
            .as_ref()
            .map(|d| format!("  {d}"))
            .unwrap_or_default();
        match step.state {
            StepState::Pending => None,
            StepState::Running => {
                self.progress_at = now;
                None
            }
            StepState::Done => Some(format!(
                "{} {name}{}",
                paint.success("✔"),
                paint.muted(&detail)
            )),
            StepState::Warned => Some(paint.warn(&format!("▲ {name}{detail}"))),
            // The full error follows as its own log line.
            StepState::Failed => {
                self.failed = true;
                Some(paint.error(&format!("✘ {name}")))
            }
            StepState::Skipped if self.failed => None,
            StepState::Skipped => Some(paint.muted(&format!("– {name}{detail}"))),
        }
    }

    fn progress(&mut self, progress: &OpProgress, now: Instant) -> Option<String> {
        if now.duration_since(self.progress_at) < PROGRESS_EVERY {
            return None;
        }
        self.progress_at = now;
        let amount = amount(progress.done, progress.total, progress.unit);
        Some(self.paint.muted(&format!("… {} {amount}", progress.label)))
    }
}

fn amount(done: u64, total: u64, unit: Unit) -> String {
    let show = |n| match unit {
        Unit::Bytes => human::bytes(n),
        Unit::Items => n.to_string(),
        Unit::Seconds => format!("{n}s"),
    };
    if total == 0 {
        show(done)
    } else {
        format!("{} / {} ({}%)", show(done), show(total), done * 100 / total)
    }
}

/// The release list as aligned columns, for people and for `awk`.
pub fn table(listing: &ReleasesListResult) -> String {
    let mut rows = vec![[
        "RELEASE".to_string(),
        "VERSION".into(),
        "COMMIT".into(),
        "BUILT (UTC)".into(),
        "SIZE".into(),
        "STATE".into(),
        "DEPLOYED BY".into(),
    ]];
    for entry in &listing.entries {
        let (version, commit, built) = match &entry.name {
            Some(name) => (
                name.version.clone(),
                format!("{}{}", name.git, if name.dirty { "*" } else { "" }),
                built_at(&name.built_at),
            ),
            None => ("-".into(), "-".into(), "-".into()),
        };
        let state = match (entry.current, entry.kind, entry.name.is_some()) {
            (true, _, _) => "live",
            (_, _, false) => "not-ours",
            (_, ReleaseKind::Open, _) => "unpacked",
            (_, ReleaseKind::Compressed, _) => "compressed",
        };
        rows.push([
            entry.id.clone(),
            version,
            commit,
            built,
            entry.size_bytes.map_or("-".into(), human::bytes),
            state.into(),
            entry
                .manifest
                .as_ref()
                .map_or("-".into(), |m| m.deployed_by.clone()),
        ]);
    }
    let widths: Vec<usize> = (0..rows[0].len())
        .map(|col| {
            rows.iter()
                .map(|row| row[col].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// `2026-10-14 15:30` in UTC, or `-` when the engine sent no time.
fn built_at(text: &str) -> String {
    DateTime::parse_from_rfc3339(text).map_or_else(
        |_| "-".to_string(),
        |at| at.format("%Y-%m-%d %H:%M").to_string(),
    )
}

/// ANSI colors on stderr, unless it is not a terminal or `NO_COLOR` is set.
#[derive(Debug, Clone, Copy)]
pub struct Paint {
    on: bool,
}

impl Paint {
    fn stderr() -> Self {
        Self {
            on: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn plain() -> Self {
        Self { on: false }
    }

    fn wrap(self, code: &str, text: &str) -> String {
        if self.on && !text.is_empty() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    fn step(self, text: &str) -> String {
        self.wrap("1;36", text)
    }
    fn command(self, text: &str) -> String {
        self.wrap("35", text)
    }
    fn info(self, text: &str) -> String {
        self.wrap("34", text)
    }
    fn success(self, text: &str) -> String {
        self.wrap("1;32", text)
    }
    fn warn(self, text: &str) -> String {
        self.wrap("33", text)
    }
    fn error(self, text: &str) -> String {
        self.wrap("1;31", text)
    }
    fn muted(self, text: &str) -> String {
        self.wrap("2", text)
    }
}

#[cfg(test)]
#[path = "headless_test.rs"]
mod tests;
