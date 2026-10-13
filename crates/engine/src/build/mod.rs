//! Running the build command on this machine and streaming its output.

mod binary;
mod version;

pub use binary::{BinaryInfo, GlibcVersion, inspect, normalize_arch};
pub use version::{deployer, release_name};

use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::config::Deployable;
use crate::deploy::report::{LogKind, Reporter, Unit};

/// Runs `build.command` in `build.cwd`. Cargo's progress bar becomes the
/// step's progress; every other line goes to the log.
pub async fn run(deployable: &Deployable, report: &Reporter, step: usize) -> Result<()> {
    let command = &deployable.config.build.command;
    let dir = deployable.build_dir();
    report.log(
        LogKind::Command,
        format!("{}  (in {})", command.join(" "), dir.display()),
    );

    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .current_dir(&dir)
        // Cargo draws its progress bar only for a terminal unless told otherwise.
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "120")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting `{}` in {}", command[0], dir.display()))?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (out, err) = tokio::join!(
        forward(stdout, LogKind::Out, report, step),
        forward(stderr, LogKind::Err, report, step),
    );
    out?;
    err?;

    let status = child.wait().await.context("waiting for the build")?;
    if !status.success() {
        match status.code() {
            Some(code) => bail!("build failed with exit code {code}"),
            None => bail!("build was killed by a signal"),
        }
    }
    Ok(())
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
            .context("reading build output")?;
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

#[cfg(test)]
#[path = "build_test.rs"]
mod tests;
