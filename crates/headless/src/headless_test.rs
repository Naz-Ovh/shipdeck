use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex};

use shipdeck_protocol::connection::Connection;
use shipdeck_protocol::message::{
    DeployableInfo, Event, LogKind, OnFailure, OpLog, OpProgress, OpStep, Outcome, ReleaseInfo,
    ReleaseKind, ReleaseName, ReleasesListResult, StepState, Unit,
};

use super::{Choices, Headless, Lines, Paint, exit_code, find, summary, table};

fn deployable(slug: &str, name: &str) -> DeployableInfo {
    DeployableInfo {
        slug: slug.into(),
        name: name.into(),
        host: "203.0.113.10".into(),
        user: Some("deploy".into()),
        port: 22,
        dir: format!("/opt/{slug}"),
    }
}

fn choices() -> Choices {
    Choices {
        trust_new_host: false,
        allow_unchanged: false,
        on_failure: OnFailure::Rollback,
    }
}

#[test]
fn every_outcome_has_its_own_exit_code() {
    let error = String::from("http 502");
    let codes = [
        Outcome::Succeeded {
            release: "r".into(),
        },
        Outcome::Failed {
            error: error.clone(),
        },
        Outcome::RolledBack {
            error: error.clone(),
            to: "r".into(),
        },
        Outcome::Kept {
            release: "r".into(),
            error: error.clone(),
        },
        Outcome::RollbackFailed {
            error: error.clone(),
            rollback_error: error,
        },
        Outcome::Unchanged { live: "r".into() },
    ]
    .map(|outcome| exit_code(&outcome));
    assert_eq!(codes, [0, 1, 3, 4, 5, 6]);
}

#[test]
fn finds_by_folder_then_by_name() {
    let all = [
        deployable("api-prod", "my-api"),
        deployable("web", "my-web"),
    ];
    assert_eq!(find(&all, "api-prod").unwrap().slug, "api-prod");
    assert_eq!(find(&all, "my-web").unwrap().slug, "web");
    let err = find(&all, "nope").unwrap_err();
    assert!(err.contains("api-prod, web"), "{err}");
}

#[test]
fn a_name_shared_by_two_needs_the_folder() {
    let all = [
        deployable("api-prod", "my-api"),
        deployable("api-staging", "my-api"),
    ];
    let err = find(&all, "my-api").unwrap_err();
    assert!(err.contains("api-prod, api-staging"), "{err}");
    assert_eq!(find(&all, "api-staging").unwrap().slug, "api-staging");
}

#[test]
fn steps_print_when_they_end() {
    let start = Instant::now();
    let steps = ["Build".to_string(), "Upload".to_string()];
    let mut lines = Lines::new(&steps, Paint::plain(), start);
    let step = |step, state, detail: Option<&str>| {
        Event::Step(OpStep {
            op: 1,
            step,
            state,
            detail: detail.map(str::to_string),
        })
    };
    assert_eq!(lines.line(step(0, StepState::Running, None), start), None);
    assert_eq!(
        lines.line(step(0, StepState::Done, Some("31.0 MiB")), start),
        Some("✔ Build  31.0 MiB".into())
    );
    assert_eq!(
        lines.line(step(1, StepState::Failed, Some("timed out")), start),
        Some("✘ Upload".into())
    );
    // After a failure the rest are skipped; that is already clear.
    assert_eq!(
        lines.line(step(1, StepState::Skipped, Some("stopped")), start),
        None
    );
}

#[test]
fn logs_keep_their_marker() {
    let now = Instant::now();
    let mut lines = Lines::new(&[], Paint::plain(), now);
    let log = |kind, text: &str| {
        Event::Log(OpLog {
            op: 1,
            kind,
            text: text.into(),
        })
    };
    assert_eq!(
        lines.line(log(LogKind::Command, "cargo build --release"), now),
        Some("› cargo build --release".into())
    );
    assert_eq!(
        lines.line(log(LogKind::Out, "Compiling my-api"), now),
        Some("  Compiling my-api".into())
    );
    assert_eq!(
        lines.line(log(LogKind::Error, "http 502"), now),
        Some("✘ http 502".into())
    );
}

#[test]
fn progress_prints_every_few_seconds() {
    let start = Instant::now();
    let steps = ["Upload".to_string()];
    let mut lines = Lines::new(&steps, Paint::plain(), start);
    let progress = |done| {
        Event::Progress(OpProgress {
            op: 1,
            step: 0,
            done,
            total: 4 * 1024 * 1024,
            unit: Unit::Bytes,
            label: "uploading".into(),
        })
    };
    assert_eq!(
        lines.line(progress(1024), start + Duration::from_secs(1)),
        None
    );
    assert_eq!(
        lines.line(progress(1024 * 1024), start + Duration::from_secs(6)),
        Some("… uploading 1.0 MiB / 4.0 MiB (25%)".into())
    );
    assert_eq!(
        lines.line(progress(2 * 1024 * 1024), start + Duration::from_secs(8)),
        None
    );
}

#[test]
fn the_summary_says_what_is_live() {
    let took = Duration::from_secs(72);
    let paint = Paint::plain();
    assert_eq!(
        summary(
            paint,
            "my-api",
            &Outcome::Succeeded {
                release: "v0.3.1".into()
            },
            took
        ),
        "✔ my-api: v0.3.1 is live (1m12s)"
    );
    assert_eq!(
        summary(
            paint,
            "my-api",
            &Outcome::RolledBack {
                error: "http 502".into(),
                to: "v0.3.0".into()
            },
            took
        ),
        "▲ my-api: the new release failed; v0.3.0 is live again"
    );
    assert_eq!(
        summary(
            paint,
            "my-api",
            &Outcome::Unchanged {
                live: "v0.3.1".into()
            },
            took
        ),
        "▲ my-api: nothing changed since v0.3.1, so nothing was deployed; \
         --allow-unchanged deploys it anyway"
    );
}

#[test]
fn the_release_table_lines_up() {
    let ids = [
        "v0.3.1-ab12cd3-20261014.153012",
        "v0.3.0-9f8e7d6-20261012.091000",
    ];
    let mut entries: Vec<ReleaseInfo> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| ReleaseInfo {
            id: id.to_string(),
            name: Some(ReleaseName {
                version: if i == 0 { "0.3.1" } else { "0.3.0" }.into(),
                git: if i == 0 { "ab12cd3" } else { "9f8e7d6" }.into(),
                dirty: false,
                built_at: if i == 0 {
                    "2026-10-14T15:30:12+00:00"
                } else {
                    "2026-10-12T09:10:00+00:00"
                }
                .into(),
            }),
            kind: if i == 0 {
                ReleaseKind::Open
            } else {
                ReleaseKind::Compressed
            },
            current: i == 0,
            size_bytes: Some(31 * 1024 * 1024),
            backup: true,
            manifest: None,
        })
        .collect();
    entries.push(ReleaseInfo {
        id: "old".into(),
        name: None,
        kind: ReleaseKind::Open,
        current: false,
        size_bytes: None,
        backup: false,
        manifest: None,
    });
    let text = table(&ReleasesListResult {
        current: Some(ids[0].into()),
        entries,
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("RELEASE  "));
    assert!(lines[1].contains("live"), "{text}");
    assert!(lines[1].contains("2026-10-14 15:30"), "{text}");
    assert!(lines[2].contains("compressed"), "{text}");
    assert!(
        lines[3].starts_with("old ") && lines[3].contains("not-ours"),
        "{text}"
    );
    // Every column starts at the same place on every line.
    let col = lines[0].find("VERSION").unwrap();
    assert!(lines[1][col..].starts_with("0.3.1"), "{text}");
}

/// A stand-in for the engine: the app side of one pipe, reading what the
/// command asks and writing what the engine would send.
struct FakeEngine {
    lines: BufReader<DuplexStream>,
    out: DuplexStream,
}

impl FakeEngine {
    /// A command, and the engine it talks to.
    fn pair(choices: Choices) -> (Headless, Self) {
        let (engine_read, app_write) = duplex(64 * 1024);
        let (app_read, engine_write) = duplex(64 * 1024);
        let connection = Connection::start(app_read, app_write);
        (
            Headless::new(connection, choices),
            Self {
                lines: BufReader::new(engine_read),
                out: engine_write,
            },
        )
    }

    /// The next line the command wrote.
    async fn next(&mut self) -> Value {
        let mut line = String::new();
        let read = self.lines.read_line(&mut line).await.unwrap();
        assert!(read > 0, "the command closed the stream");
        serde_json::from_str(&line).unwrap()
    }

    async fn send(&mut self, message: Value) {
        let line = serde_json::to_string(&message).unwrap();
        self.out.write_all(line.as_bytes()).await.unwrap();
        self.out.write_all(b"\n").await.unwrap();
        self.out.flush().await.unwrap();
    }

    /// Answers the request `asked` and returns nothing.
    async fn answer(&mut self, asked: &Value, result: Value) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": asked["id"],
            "result": result
        }))
        .await;
    }
}

#[tokio::test]
async fn a_deploy_says_how_it_ended() {
    let (mut headless, mut engine) = FakeEngine::pair(choices());
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        assert_eq!(asked["method"], "deploy/start");
        assert_eq!(asked["params"]["deployable"], "api");
        assert_eq!(asked["params"]["on_failure"], "rollback");
        engine
            .answer(
                &asked,
                json!({"op": 1, "steps": ["Preflight", "Build", "Health"]}),
            )
            .await;
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "method": "op/step",
                "params": {"op": 1, "step": 1, "state": "done", "detail": "24 files"}
            }))
            .await;
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "method": "op/finished",
                "params": {"op": 1, "outcome": {"kind": "succeeded", "release": "v0.3.1"}}
            }))
            .await;
    });

    let projects = [deployable("api", "My API")];
    let code = headless.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn a_failed_deploy_keeps_its_exit_code() {
    let (mut headless, mut engine) = FakeEngine::pair(choices());
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        engine
            .answer(&asked, json!({"op": 1, "steps": ["Preflight", "Build"]}))
            .await;
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "method": "op/finished",
                "params": {"op": 1, "outcome": {"kind": "failed", "error": "the build failed"}}
            }))
            .await;
    });

    let projects = [deployable("api", "My API")];
    let code = headless.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::from(1));
}

#[tokio::test]
async fn an_unchanged_question_is_answered_from_the_flag() {
    let mut choices = choices();
    choices.allow_unchanged = true;
    let (mut headless, mut engine) = FakeEngine::pair(choices);
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        engine
            .answer(&asked, json!({"op": 1, "steps": ["Pack"]}))
            .await;
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "ask/unchanged",
                "params": {"op": 1, "release": "v2", "live": "v1"}
            }))
            .await;
        let answer = engine.next().await;
        assert_eq!(answer["id"], 9);
        assert_eq!(answer["result"], true);
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "method": "op/finished",
                "params": {"op": 1, "outcome": {"kind": "succeeded", "release": "v2"}}
            }))
            .await;
    });

    let projects = [deployable("api", "My API")];
    let code = headless.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn a_rollback_question_is_answered_from_the_flag() {
    let mut choices = choices();
    choices.on_failure = OnFailure::Keep;
    let (mut headless, mut engine) = FakeEngine::pair(choices);
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        engine
            .answer(&asked, json!({"op": 1, "steps": ["Health"]}))
            .await;
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "ask/rollback",
                "params": {
                    "op": 1,
                    "release": "v2",
                    "previous": "v1",
                    "step": "Health",
                    "error": "http 502",
                    "seconds": 30
                }
            }))
            .await;
        let answer = engine.next().await;
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["result"], false);
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "method": "op/finished",
                "params": {"op": 1, "outcome": {"kind": "kept", "release": "v2", "error": "http 502"}}
            }))
            .await;
    });

    let projects = [deployable("api", "My API")];
    let code = headless.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::from(4));
}

#[tokio::test]
async fn a_secret_question_is_refused_with_a_hint() {
    let (mut headless, mut engine) = FakeEngine::pair(choices());
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        engine
            .answer(&asked, json!({"op": 1, "steps": ["Preflight"]}))
            .await;
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "ask/secret",
                "params": {"title": "SSH password", "prompt": "Password for prod"}
            }))
            .await;
        let answer = engine.next().await;
        assert_eq!(answer["id"], 4);
        assert_eq!(answer["result"], json!({}));
        engine
            .send(json!({
                "jsonrpc": "2.0",
                "method": "op/finished",
                "params": {"op": 1, "outcome": {"kind": "failed", "error": "login cancelled"}}
            }))
            .await;
    });

    let projects = [deployable("api", "My API")];
    let code = headless.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::from(1));
}

#[tokio::test]
async fn the_release_list_comes_from_the_engine() {
    let (mut headless, mut engine) = FakeEngine::pair(choices());
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        assert_eq!(asked["method"], "releases/list");
        assert_eq!(asked["params"]["deployable"], "api");
        engine
            .answer(
                &asked,
                json!({
                    "current": "v1",
                    "entries": [{
                        "id": "v1",
                        "name": {"version": "0.3.1", "git": "ab12cd3", "dirty": false,
                                 "built_at": "2026-10-14T15:30:12+00:00"},
                        "kind": "open",
                        "current": true,
                        "size_bytes": 1024,
                        "backup": false
                    }]
                }),
            )
            .await;
    });

    let projects = [deployable("api", "My API")];
    let code = headless.releases(&projects, "api").await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn a_name_that_is_not_there_stops_before_the_engine() {
    let (mut headless, _engine) = FakeEngine::pair(choices());
    let projects = [deployable("api", "My API")];
    let code = headless.deploy(&projects, &["nope".into()]).await;
    assert_eq!(code, ExitCode::FAILURE);
}
