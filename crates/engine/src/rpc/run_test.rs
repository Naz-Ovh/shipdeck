use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex};

use shipdeck_protocol::connection::Connection;
use shipdeck_protocol::message::common::Login;
use shipdeck_protocol::message::request::{Initialize, WorkspaceOpen};

use super::run;

/// A `shipdeck.yaml` with nothing to complain about.
const SAMPLE: &str = "\
name: Demo
dir: /opt/demo
ssh: { host: example.com, user: deploy }
service: { name: demo, start: node dist/main.js }
pipeline:
  pack:
    files:
      include:
        project: [dist]
";

/// What an app sees: its end of the two pipes the engine talks on.
struct App {
    lines: BufReader<DuplexStream>,
    out: DuplexStream,
}

impl App {
    /// Starts an engine on two in-memory pipes.
    fn start() -> Self {
        let (engine_read, app_write) = duplex(64 * 1024);
        let (app_read, engine_write) = duplex(64 * 1024);
        tokio::spawn(run(engine_read, engine_write));
        Self {
            lines: BufReader::new(app_read),
            out: app_write,
        }
    }

    async fn send(&mut self, message: Value) {
        let line = serde_json::to_string(&message).unwrap();
        self.out.write_all(line.as_bytes()).await.unwrap();
        self.out.write_all(b"\n").await.unwrap();
        self.out.flush().await.unwrap();
    }

    /// The next line the engine wrote.
    async fn next(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.lines.read_line(&mut line).await {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(&line).ok(),
        }
    }

    /// Asks `method`, skips the events, and returns its answer.
    async fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await;
        loop {
            let answer = self.next().await.expect("the engine closed the stream");
            if answer["id"] == id {
                return answer;
            }
        }
    }
}

/// An empty folder for one test, so tests do not share one.
fn scratch(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("shipdeck-rpc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A workspace of one project, and the app after opening it.
async fn opened(name: &str) -> (App, std::path::PathBuf) {
    let root = scratch(name);
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/shipdeck.yaml"), SAMPLE).unwrap();
    let mut app = App::start();
    let answer = app
        .call(
            1,
            "workspace/open",
            json!({"root": root.to_str().unwrap(), "login": {"kind": "auto"}}),
        )
        .await;
    assert_eq!(answer["result"]["problems"], json!([]));
    (app, root)
}

#[tokio::test]
async fn initialize_agrees_on_the_version() {
    let mut app = App::start();
    let answer = app
        .call(
            1,
            "initialize",
            json!({"protocol": 1, "client": "shipdeck-tui 0.7.0"}),
        )
        .await;
    let result = &answer["result"];
    assert_eq!(result["protocol"], 1);
    assert_eq!(result["shipdeck"], env!("CARGO_PKG_VERSION"));
    assert_eq!(result["capabilities"], json!([]));
}

#[tokio::test]
async fn a_protocol_this_engine_does_not_speak_is_refused() {
    let mut app = App::start();
    let answer = app
        .call(1, "initialize", json!({"protocol": 2, "client": "future"}))
        .await;
    assert_eq!(answer["error"]["code"], -32008);
}

#[tokio::test]
async fn a_project_before_the_workspace_is_refused() {
    let mut app = App::start();
    let answer = app
        .call(1, "status/get", json!({"deployable": "web"}))
        .await;
    assert_eq!(answer["error"]["code"], -32007);
}

#[tokio::test]
async fn a_method_this_engine_has_not_is_not_found() {
    let mut app = App::start();
    let answer = app.call(1, "db/tables", json!({"deployable": "web"})).await;
    assert_eq!(answer["error"]["code"], -32601);
}

#[tokio::test]
async fn params_that_do_not_fit_are_refused() {
    let mut app = App::start();
    let answer = app.call(1, "status/get", json!({"project": "web"})).await;
    assert_eq!(answer["error"]["code"], -32602);
}

#[tokio::test]
async fn the_workspace_lists_its_projects() {
    let (mut app, root) = opened("lists").await;
    let answer = app
        .call(
            2,
            "workspace/open",
            json!({"root": root.to_str().unwrap(), "login": {"kind": "auto"}}),
        )
        .await;
    let project = &answer["result"]["deployables"][0];
    assert_eq!(project["slug"], "web");
    assert_eq!(project["name"], "Demo");
    assert_eq!(project["host"], "example.com");
    assert_eq!(project["user"], "deploy");
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_folder_that_is_not_there_is_refused() {
    let mut app = App::start();
    let answer = app
        .call(
            1,
            "workspace/open",
            json!({"root": "/nonexistent", "login": {"kind": "auto"}}),
        )
        .await;
    assert_eq!(answer["error"]["code"], -32602);
}

#[tokio::test]
async fn a_project_that_is_not_there_is_unknown() {
    let (mut app, root) = opened("unknown").await;
    let answer = app
        .call(2, "deploy/start", json!({"deployable": "api"}))
        .await;
    assert_eq!(answer["error"]["code"], -32001);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn stopping_a_log_that_is_not_running_is_fine() {
    let mut app = App::start();
    let answer = app.call(1, "logs/stop", json!({"view": 7})).await;
    assert_eq!(answer["result"], json!({}));
}

#[tokio::test]
async fn an_event_from_the_app_is_ignored() {
    let mut app = App::start();
    app.send(json!({
        "jsonrpc": "2.0",
        "method": "op/log",
        "params": {"op": 1, "kind": "info", "text": "hello"}
    }))
    .await;
    let answer = app
        .call(
            1,
            "initialize",
            json!({"protocol": 1, "client": "shipdeck-tui"}),
        )
        .await;
    assert_eq!(answer["result"]["protocol"], 1);
}

#[tokio::test]
async fn a_question_from_the_app_is_refused() {
    let mut app = App::start();
    let answer = app
        .call(
            1,
            "ask/secret",
            json!({"title": "SSH password", "prompt": "Password for prod"}),
        )
        .await;
    assert_eq!(answer["error"]["code"], -32601);
}

#[tokio::test]
async fn shutdown_answers_and_ends_the_stream() {
    let mut app = App::start();
    let answer = app.call(1, "shutdown", json!({})).await;
    assert_eq!(answer["result"], json!({}));
    assert!(app.next().await.is_none());
}

#[tokio::test]
async fn a_request_sent_before_the_stream_closed_is_answered() {
    let mut app = App::start();
    app.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocol": 1, "client": "shipdeck-tui"}
    }))
    .await;
    // The app closes its side at once; the answer must still come out.
    app.out.shutdown().await.unwrap();
    let answer = app.next().await.expect("the answer to the last request");
    assert_eq!(answer["result"]["protocol"], 1);
    assert!(app.next().await.is_none());
}

#[tokio::test]
async fn an_app_using_the_protocol_crate_talks_to_the_engine() {
    let (engine_read, app_write) = duplex(64 * 1024);
    let (app_read, engine_write) = duplex(64 * 1024);
    tokio::spawn(run(engine_read, engine_write));
    let mut app = Connection::start(app_read, app_write);

    let hello = app
        .initialize(&Initialize {
            protocol: 1,
            client: "shipdeck-tui 0.7.0".into(),
        })
        .await
        .unwrap();
    assert_eq!(hello.protocol, 1);
    assert_eq!(hello.shipdeck, env!("CARGO_PKG_VERSION"));

    let root = scratch("typed");
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/shipdeck.yaml"), SAMPLE).unwrap();
    let opened = app
        .workspace_open(&WorkspaceOpen {
            root: root.to_str().unwrap().to_string(),
            login: Login::Auto,
            no_shell_env: false,
        })
        .await
        .unwrap();
    assert_eq!(opened.deployables.len(), 1);
    assert_eq!(opened.deployables[0].slug, "web");
    assert_eq!(opened.problems, Vec::new());

    app.shutdown().await.unwrap();
    assert!(app.recv().await.is_none(), "the engine said goodbye");
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_request_after_the_open_sees_the_workspace() {
    let root = scratch("order");
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/shipdeck.yaml"), SAMPLE).unwrap();
    let mut app = App::start();
    app.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "workspace/open",
        "params": {"root": root.to_str().unwrap(), "login": {"kind": "auto"}}
    }))
    .await;
    app.send(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "deploy/start",
        "params": {"deployable": "nope"}
    }))
    .await;

    assert_eq!(app.next().await.unwrap()["id"], 1);
    let second = app.next().await.unwrap();
    assert_eq!(second["id"], 2);
    // The workspace is open by now, so the project is unknown, not locked.
    assert_eq!(second["error"]["code"], -32001);
    std::fs::remove_dir_all(root).unwrap();
}
