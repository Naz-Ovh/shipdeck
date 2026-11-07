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
    let out = run(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"agent/hello","params":{"protocol":1,"version":"0.7.1"}}"#,
    ]);
    assert!(out.contains(r#""id":1"#), "{out}");
    assert!(out.contains(r#""protocol":1"#), "{out}");
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");
}
