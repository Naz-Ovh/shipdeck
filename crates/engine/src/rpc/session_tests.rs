use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream, duplex};

use shipdeck_protocol::connection::Connection;
use shipdeck_protocol::message::common::Login;
use shipdeck_protocol::message::request::{Done, WorkspaceOpen};
use shipdeck_protocol::wire::Id;

use super::Session;
use super::convert;

const DEPLOY: &str = "\
name: product
server: { ssh: { host: example.com, user: deploy }, dir: /opt/product }
apps:
  web:
    process: { start: node server.js, systemd: {} }
";

/// A session, its connection (kept alive by the caller), and the app's end of
/// the pipe for reading what it writes.
fn session() -> (Session, Connection, BufReader<DuplexStream>) {
    let (engine_read, _app_write) = duplex(64 * 1024);
    let (app_read, engine_write) = duplex(64 * 1024);
    let connection = Connection::start(engine_read, engine_write);
    let session = Session::new(connection.handle());
    (session, connection, BufReader::new(app_read))
}

#[tokio::test]
async fn a_second_run_is_busy() {
    let (session, _connection, _lines) = session();
    let op = session.begin().unwrap();
    let error = session.begin().unwrap_err();
    assert_eq!(error.code, -32006);
    session.end(op);
    assert!(session.begin().is_ok(), "the engine is free again");
}

#[tokio::test]
async fn a_project_needs_the_workspace_first() {
    let (session, _connection, _lines) = session();
    assert_eq!(session.deployment("web").unwrap_err().code, -32007);
    let error = session.pool().err().expect("the workspace is not open");
    assert_eq!(error.code, -32007);
}

#[tokio::test]
async fn a_reply_is_written_for_the_app() {
    let (session, _connection, mut lines) = session();
    session.reply(Id::Number(4), convert::value(&Done {}));

    let mut line = String::new();
    lines.read_line(&mut line).await.unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(answer["id"], 4);
    assert_eq!(answer["result"], json!({}));
}

#[tokio::test]
async fn opens_a_project_file_and_its_deploys() {
    let folder = std::env::temp_dir().join(format!("shipdeck-rpc-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("deploy")).unwrap();
    std::fs::write(
        folder.join("shipdeck.yaml"),
        "deploys:\n  production: deploy/production.yaml\n",
    )
    .unwrap();
    std::fs::write(folder.join("deploy/production.yaml"), DEPLOY).unwrap();
    let (session, _connection, _lines) = session();
    let opened = session
        .open(&WorkspaceOpen {
            root: folder.display().to_string(),
            shipdeckfile: None,
            login: Login::Auto,
            no_shell_env: true,
        })
        .await
        .unwrap();
    assert_eq!(opened.problems, Vec::new());
    assert_eq!(opened.deployables.len(), 1);
    assert_eq!(opened.deployables[0].slug, "production");
    assert_eq!(opened.deployables[0].dir, "/opt/product");
    assert_eq!(opened.deployables[0].apps, ["web"]);
    assert_eq!(session.deployment("production").unwrap().name(), "product");
    session.close().await;
    std::fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
async fn a_named_deploy_file_needs_no_project_file() {
    let folder = std::env::temp_dir().join(format!("shipdeck-rpc-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("production.yaml"), DEPLOY).unwrap();
    let (session, _connection, _lines) = session();
    let opened = session
        .open(&WorkspaceOpen {
            root: folder.display().to_string(),
            shipdeckfile: Some(folder.join("production.yaml").display().to_string()),
            login: Login::Auto,
            no_shell_env: true,
        })
        .await
        .unwrap();
    assert_eq!(opened.deployables.len(), 1);
    assert_eq!(session.deployment("product").unwrap().dir(), "/opt/product");
    session.close().await;
    std::fs::remove_dir_all(folder).unwrap();
}
