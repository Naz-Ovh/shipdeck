//! Running commands on this machine and streaming their output.

mod binary;
#[cfg(windows)]
mod gitshell;
mod version;

pub use binary::{BinaryInfo, GlibcVersion, inspect, normalize_arch};
#[cfg(windows)]
pub use gitshell::set_gitshell_path;
#[cfg(windows)]
use gitshell::sh;
pub use version::{deployer, read_version, release_name, release_name_in};

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::config::Command;
use crate::deploy::report::{LogKind, Reporter, Unit};

/// Runs one command in `dir` with the runner it names. `sh` uses the sh of
/// this machine; on Windows, the one Git for Windows has. Cargo's progress
/// bar becomes the step's progress; every other line goes to the log.
///
/// # Errors
/// Returns an error when the runner cannot start or the command exits non-zero.
///
/// # Panics
/// Panics when the child's stdout or stderr is missing; both are piped.
pub async fn run_command(
    command: &Command,
    code: &str,
    dir: &Path,
    env: &[(String, String)],
    report: &Reporter,
    step: usize,
) -> Result<()> {
    let runner = match command {
        Command::Sh(_) => "sh",
        Command::Runner { runner, .. } => runner.key(),
    };
    let shown = code.trim();
    report.log(LogKind::Command, format!("{shown}  (in {})", dir.display()));
    let mut process = if runner == "sh" {
        #[cfg(unix)]
        let process = sh();
        #[cfg(windows)]
        let process = sh().await?;
        process
    } else {
        tokio::process::Command::new(runner)
    };
    let mut child = process
        .arg(match command {
            Command::Sh(_) => "-c",
            Command::Runner { runner, .. } => runner.flag(),
        })
        .arg(code)
        .current_dir(dir)
        .envs(env.iter().map(|(key, value)| (key, value)))
        // Cargo draws its progress bar only for a terminal unless told otherwise.
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "120")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting `{shown}` in {}", dir.display()))?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (out, err) = tokio::join!(
        forward(stdout, LogKind::Out, report, step),
        forward(stderr, LogKind::Err, report, step),
    );
    out?;
    err?;

    let status = child.wait().await.context("waiting for the command")?;
    if !status.success() {
        match status.code() {
            Some(code) => bail!("`{shown}` exited with code {code}"),
            None => bail!("`{shown}` was killed by a signal"),
        }
    }
    Ok(())
}

/// The shell for `sh` commands: `sh` from PATH.
#[cfg(unix)]
fn sh() -> tokio::process::Command {
    tokio::process::Command::new("sh")
}

/// Where `name` is on this machine, looking through `PATH`.
#[must_use]
pub fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for folder in std::env::split_paths(&path) {
        for extension in extensions {
            let candidate = folder.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Reads a stream, splitting on `\n` and on the `\r` cargo uses to redraw
/// its progress bar.
async fn forward(
    mut stream: impl AsyncRead + Unpin,
    kind: LogKind,
    report: &Reporter,
    step: usize,
) -> Result<()> {
    let mut buf = [0u8; 8192];
    let mut line = Vec::new();
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .context("reading command output")?;
        if n == 0 {
            break;
        }
        for &byte in &buf[..n] {
            if byte == b'\n' || byte == b'\r' {
                emit(&line, kind, report, step);
                line.clear();
            } else {
                line.push(byte);
            }
        }
    }
    emit(&line, kind, report, step);
    Ok(())
}

fn emit(raw: &[u8], kind: LogKind, report: &Reporter, step: usize) {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end();
    if text.trim().is_empty() {
        return;
    }
    match cargo_progress(text) {
        Some((done, total, crates)) => report.progress(step, done, total, Unit::Items, crates),
        None => report.log(kind, text),
    }
}

/// Parses cargo's `Building [=====>   ] 283/286: russh, shipdeck(bin)` line.
fn cargo_progress(line: &str) -> Option<(u64, u64, String)> {
    let rest = line.trim_start().strip_prefix("Building [")?;
    let (_, rest) = rest.split_once("] ")?;
    let (counts, crates) = match rest.split_once(": ") {
        Some((counts, crates)) => (counts, crates.trim()),
        None => (rest.trim(), ""),
    };
    let (done, total) = counts.split_once('/')?;
    Some((
        done.trim().parse().ok()?,
        total.trim().parse().ok()?,
        crates.to_string(),
    ))
}
