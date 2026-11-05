use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex};

use super::{Client, Incoming};
use crate::message::ask::{Ask, SecretAnswer};
use crate::message::event::Event;
use crate::message::{Initialize, LogsFollow, LogsFollowResult};

/// One end of a test pipe, pretending to be the engine.
struct Engine {
    read: BufReader<DuplexStream>,
    write: DuplexStream,
}

impl Engine {
    /// A client and an engine, talking over two in-memory pipes.
    fn pair() -> (Client, Self) {
        let (client_read, engine_write) = duplex(64 * 1024);
        let (engine_read, client_write) = duplex(64 * 1024);
        let client = Client::start(client_read, client_write);
        let engine = Self {
            read: BufReader::new(engine_read),
            write: engine_write,
        };
        (client, engine)
    }

    /// The next line the client wrote, as JSON.
    async fn next(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.read.read_line(&mut line).await {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(&line).ok(),
        }
    }

    async fn send(&mut self, value: Value) {
        let line = serde_json::to_string(&value).unwrap();
        self.write.write_all(line.as_bytes()).await.unwrap();
        self.write.write_all(b"\n").await.unwrap();
        self.write.flush().await.unwrap();
    }
}

#[tokio::test]
async fn a_request_gets_its_answer() {
    let (client, mut engine) = Engine::pair();
    let call = tokio::spawn(async move {
        client
            .initialize(&Initialize {
                protocol: 1,
                client: "shipdeck-tui 0.7.0".into(),
            })
            .await
    });

    let asked = engine.next().await.unwrap();
    assert_eq!(asked["jsonrpc"], "2.0");
    assert_eq!(asked["method"], "initialize");
    assert_eq!(asked["params"]["protocol"], 1);

    engine
        .send(json!({
            "jsonrpc": "2.0",
            "id": asked["id"],
            "result": {"protocol": 1, "shipdeck": "0.7.0", "capabilities": ["logs"]}
        }))
        .await;

    let answer = call.await.unwrap().unwrap();
    assert_eq!(answer.shipdeck, "0.7.0");
    assert_eq!(answer.capabilities, ["logs"]);
}

#[tokio::test]
async fn an_error_answer_becomes_an_rpc_error() {
    let (client, mut engine) = Engine::pair();
    let call = tokio::spawn(async move {
        client
            .logs_follow(&LogsFollow {
                deployable: "missing".into(),
            })
            .await
    });

    let asked = engine.next().await.unwrap();
    engine
        .send(json!({
            "jsonrpc": "2.0",
            "id": asked["id"],
            "error": {"code": -32001, "message": "no such project"}
        }))
        .await;

    let error = call.await.unwrap().unwrap_err();
    match error {
        super::Error::Rpc(error) => assert_eq!(error.code, -32001),
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_event_arrives_typed() {
    let (mut client, mut engine) = Engine::pair();
    engine
        .send(json!({
            "jsonrpc": "2.0",
            "method": "op/log",
            "params": {"op": 1, "kind": "command", "text": "cargo test"}
        }))
        .await;

    match client.recv().await.unwrap() {
        Incoming::Event(Event::Log(log)) => {
            assert_eq!(log.op, 1);
            assert_eq!(log.text, "cargo test");
        }
        other => panic!("expected an op/log event, got {other:?}"),
    }
}

#[tokio::test]
async fn a_question_can_be_answered() {
    let (client, mut engine) = Engine::pair();
    engine
        .send(json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "ask/secret",
            "params": {"title": "SSH password", "prompt": "Password for prod"}
        }))
        .await;

    let mut client = client;
    let Incoming::Ask { id, ask } = client.recv().await.unwrap() else {
        panic!("expected a question");
    };
    assert!(matches!(ask, Ask::Secret(_)));
    assert_eq!(id.to_string(), "9");

    client
        .answer(
            id,
            &SecretAnswer {
                secret: Some("hunter2".into()),
            },
        )
        .unwrap();

    let answer = engine.next().await.unwrap();
    assert_eq!(answer["id"], 9);
    assert_eq!(answer["result"]["secret"], "hunter2");
}

#[tokio::test]
async fn a_method_this_version_does_not_know_is_kept() {
    let (mut client, mut engine) = Engine::pair();
    engine
        .send(json!({
            "jsonrpc": "2.0",
            "method": "db/tables",
            "params": {"deployable": "my-api"}
        }))
        .await;

    match client.recv().await.unwrap() {
        Incoming::Other { id, method, .. } => {
            assert!(id.is_none());
            assert_eq!(method, "db/tables");
        }
        other => panic!("expected an unknown method, got {other:?}"),
    }
}

#[tokio::test]
async fn a_broken_line_is_reported_not_fatal() {
    let (mut client, mut engine) = Engine::pair();
    engine
        .send(json!({"jsonrpc": "2.0", "id": 1, "method": "logs/follow"}))
        .await;
    engine
        .send(json!({"jsonrpc": "2.0", "method": "logs/line", "params": {"view": 1}}))
        .await;

    match client.recv().await.unwrap() {
        Incoming::Other { id, method, .. } => {
            assert!(id.is_some(), "the engine waits for an answer");
            assert_eq!(method, "logs/follow");
        }
        other => panic!("expected the broken request as unknown, got {other:?}"),
    }
    match client.recv().await.unwrap() {
        Incoming::Malformed { id, error } => {
            assert!(id.is_none());
            assert!(error.contains("logs/line"));
        }
        other => panic!("expected a malformed event, got {other:?}"),
    }
}

#[tokio::test]
async fn the_stream_ending_is_visible() {
    let (mut client, engine) = Engine::pair();
    drop(engine);
    assert!(client.recv().await.is_none());
}

#[tokio::test]
async fn a_typed_answer_is_read() {
    let (client, mut engine) = Engine::pair();
    let call = tokio::spawn(async move {
        client
            .logs_follow(&LogsFollow {
                deployable: "my-api".into(),
            })
            .await
    });
    let asked = engine.next().await.unwrap();
    engine
        .send(json!({"jsonrpc": "2.0", "id": asked["id"], "result": {"view": 4}}))
        .await;
    assert_eq!(call.await.unwrap().unwrap(), LogsFollowResult { view: 4 });
}
