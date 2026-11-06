use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::timeout;

use shipdeck_protocol::connection::Connection;

use shipdeck_engine::deploy::report::{OpEvent, OpUpdate};
use shipdeck_engine::ssh::Question;

use super::{Inboxes, run};
use crate::app::Fetched;

/// A stand-in for the engine: the app side of one pipe.
struct Fake {
    lines: BufReader<DuplexStream>,
    out: DuplexStream,
}

/// The three channels the app reads, and the engine writing to them.
struct Pair {
    ops: UnboundedReceiver<OpUpdate>,
    questions: UnboundedReceiver<Question>,
    fetched: UnboundedReceiver<Fetched>,
    engine: Fake,
}

fn pair() -> Pair {
    let (engine_read, app_write) = duplex(64 * 1024);
    let (app_read, engine_write) = duplex(64 * 1024);
    let connection = Connection::start(app_read, app_write);
    let (ops_tx, ops) = unbounded_channel();
    let (questions_tx, questions) = unbounded_channel();
    let (fetched_tx, fetched) = unbounded_channel();
    run(
        connection,
        Inboxes {
            ops: ops_tx,
            questions: questions_tx,
            fetched: fetched_tx,
        },
    );
    Pair {
        ops,
        questions,
        fetched,
        engine: Fake {
            lines: BufReader::new(engine_read),
            out: engine_write,
        },
    }
}

impl Fake {
    async fn send(&mut self, message: Value) {
        let line = serde_json::to_string(&message).unwrap();
        self.out.write_all(line.as_bytes()).await.unwrap();
        self.out.write_all(b"\n").await.unwrap();
        self.out.flush().await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let mut line = String::new();
        self.lines.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

/// Waits a little for a channel, so a broken bridge fails instead of hanging.
async fn next<T>(channel: &mut UnboundedReceiver<T>) -> T {
    timeout(Duration::from_secs(5), channel.recv())
        .await
        .expect("the bridge sent nothing")
        .expect("the bridge closed")
}

#[tokio::test]
async fn a_step_reaches_the_operation_screen() {
    let mut pair = pair();
    pair.engine
        .send(json!({
            "jsonrpc": "2.0",
            "method": "op/step",
            "params": {"op": 3, "step": 1, "state": "done", "detail": "3.0s"}
        }))
        .await;

    let update = next(&mut pair.ops).await;
    assert_eq!(update.op, 3);
    match update.event {
        OpEvent::Step {
            step,
            state,
            detail,
        } => {
            assert_eq!(step, 1);
            assert_eq!(state, shipdeck_engine::deploy::report::StepState::Done);
            assert_eq!(detail.as_deref(), Some("3.0s"));
        }
        other => panic!("expected a step, got {other:?}"),
    }
}

#[tokio::test]
async fn a_log_line_reaches_the_log_view() {
    let mut pair = pair();
    pair.engine
        .send(json!({
            "jsonrpc": "2.0",
            "method": "logs/line",
            "params": {"view": 2, "line": "GET /health 200"}
        }))
        .await;

    match next(&mut pair.fetched).await {
        Fetched::LogLine { view, line } => {
            assert_eq!(view, 2);
            assert_eq!(line, "GET /health 200");
        }
        _ => panic!("expected a log line"),
    }
}

#[tokio::test]
async fn an_unchanged_question_is_shown_and_its_answer_goes_back() {
    let mut pair = pair();
    pair.engine
        .send(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "ask/unchanged",
            "params": {"op": 1, "release": "v2", "live": "v1"}
        }))
        .await;

    let update = next(&mut pair.ops).await;
    let OpEvent::AskUnchanged(question) = update.event else {
        panic!("expected the question as an event");
    };
    assert_eq!(question.live, "v1");
    question.reply.send(true).unwrap();

    let answer = pair.engine.next().await;
    assert_eq!(answer["id"], 7);
    assert_eq!(answer["result"], true);
}

#[tokio::test]
async fn a_secret_question_becomes_a_dialog_and_its_answer_goes_back() {
    let mut pair = pair();
    pair.engine
        .send(json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "ask/secret",
            "params": {"title": "SSH password", "prompt": "Password for prod"}
        }))
        .await;

    let Question::Secret { prompt, reply, .. } = next(&mut pair.questions).await else {
        panic!("expected a password dialog");
    };
    assert_eq!(prompt, "Password for prod");
    reply.send(Some("hunter2".into())).unwrap();

    let answer = pair.engine.next().await;
    assert_eq!(answer["id"], 9);
    assert_eq!(answer["result"]["secret"], "hunter2");
}
