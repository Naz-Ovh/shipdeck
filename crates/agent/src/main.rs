//! The agent's entry: `--version`, or the JSON-RPC loop on stdin and stdout.
//! Errors go to stderr and to /var/lib/shipdeck/agent.log.

use std::process::ExitCode;
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--receive") {
        if args.len() != 2 {
            fail("--receive needs one reservation");
            return ExitCode::FAILURE;
        }
        let Some(ticket) = args.get(1).and_then(|arg| arg.to_str()) else {
            fail("invalid reservation path");
            return ExitCode::FAILURE;
        };
        match shipdeck_agent::receive_recorded(ticket, tokio::io::stdin()).await {
            Ok(()) => {
                let mut output = tokio::io::stdout();
                if output.write_all(b"ok\n").await.is_err() || output.flush().await.is_err() {
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            Err(err) => {
                fail(&format!("{err:#}"));
                std::process::exit(1);
            }
        }
    }
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("shipdeck-agent {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match shipdeck_agent::serve(tokio::io::stdin(), tokio::io::stdout()).await {
        Ok(()) => {
            // The stdin reader holds a blocking thread that a runtime shutdown
            // would wait for; everything queued is already flushed.
            std::process::exit(0);
        }
        Err(err) => {
            fail(&format!("{err:#}"));
            std::process::exit(1);
        }
    }
}

/// Reports an error on stderr and in the agent's own log.
fn fail(message: &str) {
    eprintln!("shipdeck-agent: {message}");
    if let Err(err) = shipdeck_agent::record_error(message) {
        eprintln!("shipdeck-agent: writing the agent log: {err:#}");
    }
}
