use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex};
use tokio::sync::mpsc::unbounded_channel;

use shipdeck_protocol::connection::Connection;

use super::operation;
use crate::deploy::report::{Outcome, Reporter};

use super::Session;

/// One engine end, the app's end for answers, and a reporter for one run.
fn pair() -> (Reporter, BufReader<DuplexStream>, DuplexStream, Connection) {
    let (engine_read, app_write) = duplex(64 * 1024);
    let (app_read, engine_write) = duplex(64 * 1024);
    let connection = Connection::start(engine_read, engine_write);
    let session = Arc::new(Session::new(connection.handle()));
    let (reports, inbox) = unbounded_channel();
    operation(connection.handle(), session, 1, inbox);
    (
        Reporter::new(1, reports),
        BufReader::new(app_read),
        app_write,
        connection,
    )
}

async fn next(lines: &mut BufReader<DuplexStream>) -> Value {
    let mut line = String::new();
    lines.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

async fn send(out: &mut DuplexStream, message: Value) {
    let line = serde_json::to_string(&message).unwrap();
    out.write_all(line.as_bytes()).await.unwrap();
    out.write_all(b"\n").await.unwrap();
    out.flush().await.unwrap();
}

#[tokio::test]
async fn a_step_and_its_log_reach_the_app() {
    let (reporter, mut lines, _app, _connection) = pair();
    reporter.start(0, "Preflight: checking your PC and the server");

    let step = next(&mut lines).await;
    assert_eq!(step["method"], "op/step");
    assert_eq!(step["params"]["op"], 1);
    assert_eq!(step["params"]["step"], 0);
    assert_eq!(step["params"]["state"], "running");

    let log = next(&mut lines).await;
    assert_eq!(log["method"], "op/log");
    assert_eq!(log["params"]["kind"], "step");
    assert_eq!(
        log["params"]["text"],
        "Preflight: checking your PC and the server"
    );
}

#[tokio::test]
async fn how_a_run_ended_reaches_the_app() {
    let (reporter, mut lines, _app, _connection) = pair();
    reporter.done(Outcome::Succeeded {
        release: "v0.3.1-ab12cd3-20261014.153012".into(),
    });

    let finished = next(&mut lines).await;
    assert_eq!(finished["method"], "op/finished");
    assert_eq!(finished["params"]["op"], 1);
    assert_eq!(finished["params"]["outcome"]["kind"], "succeeded");
    assert_eq!(
        finished["params"]["outcome"]["release"],
        "v0.3.1-ab12cd3-20261014.153012"
    );
}

#[tokio::test]
async fn a_rollback_question_reaches_the_app_and_comes_back() {
    let (reporter, mut lines, mut app, _connection) = pair();
    let answer = reporter.ask_rollback("v2", "v1", "Health".to_string(), "it did not answer", 30);

    let asked = next(&mut lines).await;
    assert_eq!(asked["method"], "ask/rollback");
    assert_eq!(asked["params"]["op"], 1);
    assert_eq!(asked["params"]["release"], "v2");
    assert_eq!(asked["params"]["previous"], "v1");
    assert_eq!(asked["params"]["step"], "Health");
    assert_eq!(asked["params"]["seconds"], 30);

    // "Keep the new release."
    send(
        &mut app,
        json!({"jsonrpc": "2.0", "id": asked["id"], "result": false}),
    )
    .await;
    assert!(!answer.await.unwrap(), "the app said not to roll back");
}

#[tokio::test]
async fn an_unchanged_question_reaches_the_app_and_comes_back() {
    let (reporter, mut lines, mut app, _connection) = pair();
    let answer = reporter.ask_unchanged("v2", "v1");

    let asked = next(&mut lines).await;
    assert_eq!(asked["method"], "ask/unchanged");
    assert_eq!(asked["params"]["release"], "v2");
    assert_eq!(asked["params"]["live"], "v1");

    send(
        &mut app,
        json!({"jsonrpc": "2.0", "id": asked["id"], "result": true}),
    )
    .await;
    assert!(answer.await.unwrap(), "the app said to deploy anyway");
}
