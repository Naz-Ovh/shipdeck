use std::time::Duration;

use serde_json::Value;
use tokio::io::duplex;
use tokio::task::JoinHandle;

use shipdeck_protocol::connection::{Connection, Error};
use shipdeck_protocol::message::agent::{
    AgentHello, AgentHelloResult, AgentPing, AgentShutdown, HELLO, PING, PROTOCOL, SHUTDOWN,
};
use shipdeck_protocol::wire::code;

use super::serve;

/// A client talking to the agent over two in-memory pipes.
fn pair() -> (Connection, JoinHandle<anyhow::Result<()>>) {
    let (client_write, server_read) = duplex(64 * 1024);
    let (server_write, client_read) = duplex(64 * 1024);
    let server = tokio::spawn(serve(server_read, server_write));
    (Connection::start(client_read, client_write), server)
}

fn hello(protocol: u32) -> AgentHello {
    AgentHello {
        protocol,
        version: "0.7.1".into(),
    }
}

#[tokio::test]
async fn hello_answers_with_the_agent_version() {
    let (client, server) = pair();
    let answer = client
        .request::<_, AgentHelloResult>(HELLO, &hello(PROTOCOL))
        .await
        .expect("hello");
    assert_eq!(answer.protocol, PROTOCOL);
    assert_eq!(answer.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(answer.os, std::env::consts::OS);

    client
        .request::<_, AgentShutdown>(SHUTDOWN, &AgentShutdown {})
        .await
        .expect("shutdown");
    server.await.expect("the agent task").expect("a clean exit");
}

#[tokio::test]
async fn a_foreign_protocol_is_refused_and_ends_the_agent() {
    let (client, server) = pair();
    let err = client
        .request::<_, AgentHelloResult>(HELLO, &hello(PROTOCOL + 1))
        .await
        .expect_err("a foreign protocol");
    match err {
        Error::Rpc(object) => assert_eq!(object.code, code::PROTOCOL_VERSION),
        other => panic!("expected a protocol error, got {other}"),
    }
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("the agent exits")
        .expect("the agent task")
        .expect("a clean exit");
}

#[tokio::test]
async fn ping_answers() {
    let (client, server) = pair();
    let _: AgentPing = client.request(PING, &AgentPing {}).await.expect("ping");
    client
        .request::<_, AgentShutdown>(SHUTDOWN, &AgentShutdown {})
        .await
        .expect("shutdown");
    let _ = server.await;
}

#[tokio::test]
async fn an_unknown_method_says_so() {
    let (client, server) = pair();
    let err = client
        .request::<_, Value>("agent/missing", &serde_json::json!({}))
        .await
        .expect_err("an unknown method");
    match err {
        Error::Rpc(object) => assert_eq!(object.code, code::METHOD_NOT_FOUND),
        other => panic!("expected a method error, got {other}"),
    }
    client
        .request::<_, AgentShutdown>(SHUTDOWN, &AgentShutdown {})
        .await
        .expect("shutdown");
    let _ = server.await;
}

#[tokio::test]
async fn an_unsupported_ask_gets_an_error() {
    let (client, server) = pair();
    let err = client
        .request::<_, Value>(
            "ask/secret",
            &serde_json::json!({"title": "test", "prompt": "test"}),
        )
        .await
        .expect_err("an unsupported ask");
    match err {
        Error::Rpc(object) => assert_eq!(object.code, code::METHOD_NOT_FOUND),
        other => panic!("expected a method error, got {other}"),
    }
    client
        .request::<_, AgentShutdown>(SHUTDOWN, &AgentShutdown {})
        .await
        .expect("shutdown");
    let _ = server.await;
}

#[tokio::test]
async fn bad_params_are_refused_but_the_agent_stays() {
    let (client, server) = pair();
    let err = client
        .request::<_, AgentHelloResult>(HELLO, &serde_json::json!({"protocol": "one"}))
        .await
        .expect_err("bad params");
    match err {
        Error::Rpc(object) => assert_eq!(object.code, code::INVALID_PARAMS),
        other => panic!("expected a params error, got {other}"),
    }
    let answer = client
        .request::<_, AgentHelloResult>(HELLO, &hello(PROTOCOL))
        .await
        .expect("hello after a bad request");
    assert_eq!(answer.protocol, PROTOCOL);
    client
        .request::<_, AgentShutdown>(SHUTDOWN, &AgentShutdown {})
        .await
        .expect("shutdown");
    let _ = server.await;
}
