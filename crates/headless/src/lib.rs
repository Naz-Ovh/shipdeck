//! Deploy, rollback and the release list without the TUI, for scripts and
//! CI: progress goes to stderr as plain lines, nothing waits for a person,
//! and the exit code says how it ended.

use std::future::Future;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use shipdeck_engine::config::{DeployConfig, Deployable, Discovery, Rollback};
use shipdeck_engine::deploy;
use shipdeck_engine::deploy::report::{
    LogKind, OpEvent, OpUpdate, Outcome, Reporter, StepState, Unit,
};
use shipdeck_engine::human;
use shipdeck_engine::remote::{Kind, Listing, Server};
use shipdeck_engine::ssh::{Pool, Question};

/// Ctrl-C pressed twice: the run was stopped where it was.
const INTERRUPTED: u8 = 130;

/// How often a long step reports how far it got.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// What to do when a release fails after the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// Put the release that was live before back at once
    Rollback,
    /// Keep the failed release live, to look at it
    Keep,
}

impl OnFailure {
    /// Overrides `pipeline.health.rollback`. `keep` stays a question so
    /// the answer can be no; `rollback` needs no question at all.
    fn apply(self, config: &mut DeployConfig) {
        config.pipeline.health.rollback = match self {
            OnFailure::Rollback => Rollback::Auto,
            OnFailure::Keep => Rollback::Ask,
        };
    }
}

/// The exit code for each way an operation ends. 2 is left to the flag
/// parser's usage errors.
pub fn exit_code(outcome: &Outcome) -> u8 {
    match outcome {
        Outcome::Succeeded { .. } => 0,
        Outcome::Failed { .. } => 1,
        Outcome::RolledBack { .. } => 3,
        Outcome::Kept { .. } => 4,
        Outcome::RollbackFailed { .. } => 5,
    }
}

/// The deployable called `name`: its folder name, else its `name` field.
pub fn find<'a>(deployables: &'a [Deployable], name: &str) -> Result<&'a Deployable, String> {
    if let Some(found) = deployables.iter().find(|d| d.slug == name) {
        return Ok(found);
    }
    let named: Vec<&Deployable> = deployables
        .iter()
        .filter(|d| d.config.name == name)
        .collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => {
            let known: Vec<&str> = deployables.iter().map(|d| d.slug.as_str()).collect();
            Err(if known.is_empty() {
                format!("no deployable named {name}: the deploy root has none")
            } else {
                format!("no deployable named {name}; there is {}", known.join(", "))
            })
        }
        many => {
            let folders: Vec<&str> = many.iter().map(|d| d.slug.as_str()).collect();
            Err(format!(
                "{} deployables are named {name}; use a folder name: {}",
                many.len(),
                folders.join(", ")
            ))
        }
    }
}

pub struct Headless {
    pool: Arc<Pool>,
    questions: UnboundedReceiver<Question>,
    trust_new_host: bool,
    paint: Paint,
}

impl Headless {
    pub fn new(
        pool: Arc<Pool>,
        questions: UnboundedReceiver<Question>,
        trust_new_host: bool,
    ) -> Self {
        Self {
            pool,
            questions,
            trust_new_host,
            paint: Paint::stderr(),
        }
    }

    /// Deploys each in turn; the first that does not succeed stops the rest.
    pub async fn deploy(
        &mut self,
        discovery: &Discovery,
        names: &[String],
        on_failure: OnFailure,
    ) -> ExitCode {
        let mut chosen: Vec<Deployable> = Vec::new();
        for name in names {
            match find(&discovery.deployables, name) {
                Ok(found) if chosen.iter().any(|d| d.slug == found.slug) => {}
                Ok(found) => chosen.push(found.clone()),
                Err(err) => return self.fail(&err),
            }
        }
        for (index, mut deployable) in chosen.iter().cloned().enumerate() {
            on_failure.apply(&mut deployable.config);
            let steps = deploy::deploy_step_names(&deployable.config);
            let (tx, ops) = unbounded_channel();
            self.heading("deploy", &deployable);
            tokio::spawn(deploy::deploy(
                deployable.clone(),
                self.pool.clone(),
                Reporter::new(1, tx),
            ));
            let code = self.watch(&deployable, &steps, ops, on_failure).await;
            if code != 0 {
                let rest: Vec<&str> = chosen[index + 1..]
                    .iter()
                    .map(|d| d.slug.as_str())
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

    pub async fn rollback(&mut self, discovery: &Discovery, name: &str, to: String) -> ExitCode {
        let deployable = match find(&discovery.deployables, name) {
            Ok(found) => found.clone(),
            Err(err) => return self.fail(&err),
        };
        let steps = deploy::rollback_step_names(&deployable.config);
        let (tx, ops) = unbounded_channel();
        self.heading(&format!("rollback to {to}"), &deployable);
        tokio::spawn(deploy::rollback(
            deployable.clone(),
            to,
            self.pool.clone(),
            Reporter::new(1, tx),
        ));
        // A manual rollback that fails puts back what was live; nothing to keep.
        let code = self
            .watch(&deployable, &steps, ops, OnFailure::Rollback)
            .await;
        ExitCode::from(code)
    }

    /// Prints the releases on the server, newest first, to stdout.
    pub async fn releases(&mut self, discovery: &Discovery, name: &str) -> ExitCode {
        let deployable = match find(&discovery.deployables, name) {
            Ok(found) => found.clone(),
            Err(err) => return self.fail(&err),
        };
        let pool = self.pool.clone();
        let listing = self
            .answering(async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .list()
                    .await
            })
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
            Err(err) => self.fail(&format!("{err:#}")),
        }
    }

    /// Prints one operation's events until it ends; returns its exit code.
    async fn watch(
        &mut self,
        deployable: &Deployable,
        steps: &[&'static str],
        mut ops: UnboundedReceiver<OpUpdate>,
        on_failure: OnFailure,
    ) -> u8 {
        let mut lines = Lines::new(steps, self.paint, Instant::now());
        let started = Instant::now();
        let mut interrupted = false;
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        loop {
            tokio::select! {
                update = ops.recv() => match update.map(|u| u.event) {
                    Some(OpEvent::Finished(outcome)) => {
                        let name = &deployable.config.name;
                        eprintln!("{}", summary(self.paint, name, &outcome, started.elapsed()));
                        return exit_code(&outcome);
                    }
                    Some(OpEvent::AskRollback(question)) => {
                        // The deploy may have stopped waiting already; it
                        // logs the choice itself.
                        let _ = question.reply.send(on_failure == OnFailure::Rollback);
                    }
                    Some(event) => {
                        if let Some(line) = lines.line(event, Instant::now()) {
                            eprintln!("{line}");
                        }
                    }
                    None => {
                        eprintln!("{}", self.paint.error("✘ the operation stopped without a result"));
                        return 1;
                    }
                },
                Some(question) = self.questions.recv() => {
                    answer(self.paint, self.trust_new_host, question);
                }
                _ = &mut ctrl_c => {
                    if interrupted {
                        eprintln!("{}", self.paint.error("✘ stopped"));
                        return INTERRUPTED;
                    }
                    interrupted = true;
                    eprintln!(
                        "{}",
                        self.paint.warn(
                            "▲ stopping now can leave the server between two releases; \
                             press Ctrl-C again to stop anyway"
                        )
                    );
                    ctrl_c.set(tokio::signal::ctrl_c());
                }
            }
        }
    }

    /// Runs `work`, answering any SSH question it raises meanwhile.
    async fn answering<F: Future>(&mut self, work: F) -> F::Output {
        tokio::pin!(work);
        loop {
            tokio::select! {
                out = &mut work => return out,
                Some(question) = self.questions.recv() => {
                    answer(self.paint, self.trust_new_host, question);
                }
            }
        }
    }

    fn heading(&self, what: &str, deployable: &Deployable) {
        eprintln!(
            "{}",
            self.paint.step(&format!(
                "━━ {} · {what} → {}:{}",
                deployable.config.name,
                deployable.endpoint.label(),
                deployable.config.service.dir
            ))
        );
    }

    fn fail(&self, error: &str) -> ExitCode {
        eprintln!("{}", self.paint.error(&format!("✘ {error}")));
        ExitCode::FAILURE
    }
}

/// Nobody is there to type: a new host key is trusted only with
/// `--trust-new-host`, and a password or passphrase is never asked.
fn answer(paint: Paint, trust_new_host: bool, question: Question) {
    match question {
        Question::TrustHost {
            host,
            algorithm,
            fingerprint,
            reply,
        } => {
            if trust_new_host {
                eprintln!(
                    "{}",
                    paint.warn(&format!(
                        "▲ trusting the new host key of {host} ({algorithm} {fingerprint}) \
                         and saving it to ~/.ssh/known_hosts"
                    ))
                );
            } else {
                eprintln!(
                    "{}",
                    paint.error(&format!(
                        "✘ {host} is not in ~/.ssh/known_hosts; its key is {algorithm} {fingerprint}. \
                         If that is right, run again with --trust-new-host"
                    ))
                );
            }
            let _ = reply.send(trust_new_host);
        }
        Question::Secret { prompt, reply, .. } => {
            eprintln!(
                "{}",
                paint.error(&format!(
                    "✘ {prompt}: nothing can be asked here. Set SHIPDECK_KEY_PASSPHRASE, \
                     use a key without a passphrase, or set SHIPDECK_PASSWORD"
                ))
            );
            let _ = reply.send(None);
        }
    }
}

/// The last line of an operation: what is live now. The error itself is
/// already in the log right above it.
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
    }
}

/// Turns operation events into lines: logs as they come, each step once it
/// ends, and progress every few seconds while a long step runs.
pub struct Lines {
    steps: Vec<&'static str>,
    paint: Paint,
    /// After a failure the remaining steps are all skipped; no need to list them.
    failed: bool,
    /// When progress was last printed, or the running step started.
    progress_at: Instant,
}

impl Lines {
    pub fn new(steps: &[&'static str], paint: Paint, now: Instant) -> Self {
        Self {
            steps: steps.to_vec(),
            paint,
            failed: false,
            progress_at: now,
        }
    }

    pub fn line(&mut self, event: OpEvent, now: Instant) -> Option<String> {
        let paint = self.paint;
        match event {
            OpEvent::Log { kind, text } => Some(match kind {
                LogKind::Step => paint.step(&format!("━━ {text}")),
                LogKind::Command => paint.command(&format!("› {text}")),
                LogKind::Out | LogKind::Err => format!("  {text}"),
                LogKind::Info => paint.info(&format!("ℹ {text}")),
                LogKind::Success => paint.success(&format!("✔ {text}")),
                LogKind::Warn => paint.warn(&format!("▲ {text}")),
                LogKind::Error => paint.error(&format!("✘ {text}")),
            }),
            OpEvent::Step {
                step,
                state,
                detail,
            } => {
                let name = self.steps.get(step).copied().unwrap_or("step");
                let detail = detail.map(|d| format!("  {d}")).unwrap_or_default();
                match state {
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
            OpEvent::Progress {
                done,
                total,
                unit,
                label,
                ..
            } => {
                if now.duration_since(self.progress_at) < PROGRESS_EVERY {
                    return None;
                }
                self.progress_at = now;
                Some(paint.muted(&format!("… {label} {}", amount(done, total, unit))))
            }
            OpEvent::Release(id) => Some(paint.info(&format!("ℹ release {id}"))),
            OpEvent::AskRollback(_) | OpEvent::Finished(_) => None,
        }
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
pub fn table(listing: &Listing) -> String {
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
                name.stamp.format("%Y-%m-%d %H:%M").to_string(),
            ),
            None => ("-".into(), "-".into(), "-".into()),
        };
        let state = match (entry.current, entry.kind, entry.name.is_some()) {
            (true, _, _) => "live",
            (_, _, false) => "not-ours",
            (_, Kind::Open, _) => "unpacked",
            (_, Kind::Compressed, _) => "compressed",
        };
        rows.push([
            entry.id.clone(),
            version,
            commit,
            built,
            entry.size_bytes.map(human::bytes).unwrap_or("-".into()),
            state.into(),
            entry
                .manifest
                .as_ref()
                .map(|m| m.deployed_by.clone())
                .unwrap_or("-".into()),
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

/// ANSI colors on stderr, unless it is not a terminal or NO_COLOR is set.
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
