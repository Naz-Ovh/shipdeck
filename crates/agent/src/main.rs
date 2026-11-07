//! The agent's entry: `--version`, or the JSON-RPC loop on stdin and stdout.
//! Everything else goes to stderr.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
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
            eprintln!("shipdeck-agent: {err:#}");
            std::process::exit(1);
        }
    }
}
