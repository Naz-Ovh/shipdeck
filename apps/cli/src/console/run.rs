//! One command run against the engine: what it asks, and what it says.

use std::process::ExitCode;
use std::time::Instant;

use shipdeck_protocol::connection::{Connection, Error, Handle, Incoming};
use shipdeck_protocol::message::ask::{Ask, SecretAnswer};
use shipdeck_protocol::message::common::{DeployableInfo, OnFailure, Outcome};
use shipdeck_protocol::message::event::Event;
use shipdeck_protocol::message::method;
use shipdeck_protocol::message::request::{
    DeployStart, OperationStarted, ReleasesList, ReleasesListResult, RollbackStart,
};
use shipdeck_protocol::wire::Id;

use super::lines::{Lines, summary};
use super::paint::Paint;
use super::table::table;

/// Ctrl-C pressed twice: the run was stopped where it was.
const INTERRUPTED: u8 = 130;

/// What the flags of this run say, and so how to answer the engine's
/// questions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Choices {
    /// Trust a server's new host key without asking.
    pub(crate) trust_new_host: bool,
    /// Deploy a release the live one already holds.
    pub(crate) allow_unchanged: bool,
    /// What a failure after the switch does; overrides `pipeline.failed`.
    pub(crate) on_failure: OnFailure,
}

/// A run against the engine: one command, one folder, no questions.
pub(crate) struct Console {
    connection: Connection,
    paint: Paint,
    choices: Choices,
}

impl Console {
    #[must_use]
    pub(crate) fn new(connection: Connection, choices: Choices) -> Self {
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
pub(crate) fn exit_code(outcome: &Outcome) -> u8 {
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
pub(crate) fn find<'a>(
    projects: &'a [DeployableInfo],
    name: &str,
) -> Result<&'a DeployableInfo, String> {
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

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;
