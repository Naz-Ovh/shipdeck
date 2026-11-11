use std::process::ExitCode;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex};

use shipdeck_protocol::connection::Connection;
use shipdeck_protocol::message::{DeployableInfo, OnFailure, Outcome};

use super::{Choices, Console, exit_code, find};

fn deployable(slug: &str, name: &str) -> DeployableInfo {
    DeployableInfo {
        slug: slug.into(),
        name: name.into(),
        host: "203.0.113.10".into(),
        user: Some("deploy".into()),
        port: 22,
        dir: format!("/opt/{slug}"),
        folder: format!("deploy/{slug}"),
        apps: Vec::new(),
        ssh_from: Vec::new(),
        unavailable: None,
    }
}

fn choices() -> Choices {
    Choices {
        trust_new_host: false,
        allow_unchanged: false,
        on_failure: None,
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

/// A stand-in for the engine: the app side of one pipe, reading what the
/// command asks and writing what the engine would send.
struct FakeEngine {
    lines: BufReader<DuplexStream>,
    out: DuplexStream,
}

impl FakeEngine {
    /// A command, and the engine it talks to.
    fn pair(choices: Choices) -> (Console, Self) {
        let (engine_read, app_write) = duplex(64 * 1024);
        let (app_read, engine_write) = duplex(64 * 1024);
        let connection = Connection::start(app_read, app_write);
        (
            Console::new(connection, choices),
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
async fn release_listing_answers_host_key_questions() {
    let mut options = choices();
    options.trust_new_host = true;
    let (mut console, mut engine) = FakeEngine::pair(options);
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        assert_eq!(asked["method"], "releases/list");
        engine
            .send(json!({"jsonrpc":"2.0", "id":42, "method":"ask/trust_host",
            "params":{"host":"example", "algorithm":"ssh-ed25519", "fingerprint":"test"}}))
            .await;
        let answer = engine.next().await;
        assert_eq!(answer["id"], 42);
        assert_eq!(answer["result"], true);
        engine
            .answer(&asked, json!({"current":null,"entries":[]}))
            .await;
    });
    let projects = [deployable("api", "My API")];
    assert_eq!(console.releases(&projects, "api").await, ExitCode::SUCCESS);
    script.await.expect("fake engine");
}

#[tokio::test]
async fn a_deploy_says_how_it_ended() {
    let (mut console, mut engine) = FakeEngine::pair(choices());
    let script = tokio::spawn(async move {
        let asked = engine.next().await;
        assert_eq!(asked["method"], "deploy/start");
        assert_eq!(asked["params"]["deployable"], "api");
        assert!(
            asked["params"].get("on_failure").is_none(),
            "no flag sends no override"
        );
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
    let code = console.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn a_failed_deploy_keeps_its_exit_code() {
    let (mut console, mut engine) = FakeEngine::pair(choices());
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
    let code = console.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::from(1));
}

#[tokio::test]
async fn an_unchanged_question_is_answered_from_the_flag() {
    let mut choices = choices();
    choices.allow_unchanged = true;
    let (mut console, mut engine) = FakeEngine::pair(choices);
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
    let code = console.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn a_rollback_question_is_answered_from_the_flag() {
    let mut choices = choices();
    choices.on_failure = Some(OnFailure::Keep);
    let (mut console, mut engine) = FakeEngine::pair(choices);
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
    let code = console.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::from(4));
}

#[tokio::test]
async fn a_secret_question_is_refused_with_a_hint() {
    let (mut console, mut engine) = FakeEngine::pair(choices());
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
    let code = console.deploy(&projects, &["api".into()]).await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::from(1));
}

#[tokio::test]
async fn the_release_list_comes_from_the_engine() {
    let (mut console, mut engine) = FakeEngine::pair(choices());
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
    let code = console.releases(&projects, "api").await;
    script.await.unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn a_name_that_is_not_there_stops_before_the_engine() {
    let (mut console, _engine) = FakeEngine::pair(choices());
    let projects = [deployable("api", "My API")];
    let code = console.deploy(&projects, &["nope".into()]).await;
    assert_eq!(code, ExitCode::FAILURE);
}
