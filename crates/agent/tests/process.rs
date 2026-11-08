//! Process-level checks: run the real binary on stdin and stdout.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// Runs the agent with `lines`, closes stdin, and returns its stdout.
fn run(lines: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_shipdeck-agent"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the agent starts");
    let mut stdin = child.stdin.take().expect("stdin");
    for line in lines {
        writeln!(stdin, "{line}").expect("write a request");
    }
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":999,"method":"agent/shutdown","params":{{}}}}"#
    )
    .expect("write shutdown");
    drop(stdin);
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read the answers");
    let status = child.wait().expect("the agent exits");
    assert!(status.success(), "the agent exited with {status}");
    out
}

#[test]
fn an_unsupported_ask_gets_an_error_not_silence() {
    let out = run(&[
        r#"{"jsonrpc":"2.0","id":7,"method":"ask/secret","params":{"title":"t","prompt":"p"}}"#,
    ]);
    assert!(out.contains(r#""id":7"#), "{out}");
    assert!(out.contains("-32601"), "{out}");
}

#[test]
fn hello_answers_on_the_real_binary() {
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"agent/hello","params":{{"protocol":{},"version":"{}"}}}}"#,
        shipdeck_protocol::message::agent::PROTOCOL,
        env!("CARGO_PKG_VERSION")
    );
    let out = run(&[&request]);
    assert!(out.contains(r#""id":1"#), "{out}");
    assert!(
        out.contains(&format!(
            r#""protocol":{}"#,
            shipdeck_protocol::message::agent::PROTOCOL
        )),
        "{out}"
    );
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");
}
