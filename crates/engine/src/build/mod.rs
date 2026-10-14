//! Running commands on this machine (the build and the local hooks) and
//! streaming their output.

mod binary;
mod version;

pub use binary::{BinaryInfo, GlibcVersion, inspect, normalize_arch};
pub use version::{deployer, read_version, release_name};

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::config::{CommandLine, Deployable};
use crate::deploy::report::{LogKind, Reporter, Unit};

/// Runs `build.command` in `build.cwd`, if there is one.
pub async fn run(deployable: &Deployable, report: &Reporter, step: usize) -> Result<()> {
    let Some(command) = &deployable.config.build.command else {
        report.log(
            LogKind::Info,
            "no build.command; shipping the files as they are",
        );
        return Ok(());
    };
    let env = local_env(deployable, None);
    run_local(command, &deployable.build_dir(), &env, report, step)
        .await
        .context("the build failed")
}

/// What every local command sees besides the usual environment.
pub fn local_env(deployable: &Deployable, release: Option<&str>) -> Vec<(&'static str, String)> {
    let folder = std::path::absolute(&deployable.folder).unwrap_or(deployable.folder.clone());
    let mut env = vec![
        ("SHIPDECK_NAME", deployable.config.name.clone()),
        ("SHIPDECK_FOLDER", folder.display().to_string()),
        ("SHIPDECK_HOST", deployable.endpoint.name.clone()),
        ("SHIPDECK_APP_DIR", deployable.config.remote.dir.clone()),
    ];
    if let Some(id) = release {
        env.push(("SHIPDECK_RELEASE", id.to_string()));
    }
    env
}

/// Runs one command in `dir`. Cargo's progress bar becomes the step's
/// progress; every other line goes to the log.
pub async fn run_local(
    command: &CommandLine,
    dir: &Path,
    env: &[(&str, String)],
    report: &Reporter,
    step: usize,
) -> Result<()> {
    report.log(
        LogKind::Command,
        format!("{}  (in {})", command.display(), dir.display()),
    );
    let mut process = match command {
        CommandLine::Shell(line) => {
            let mut process = Command::new("sh");
            process.arg("-c").arg(line);
            process
        }
        CommandLine::Args(args) => {
            let mut process = Command::new(&args[0]);
            process.args(&args[1..]);
            process
        }
    };
    let program = match command {
        CommandLine::Shell(_) => "sh",
        CommandLine::Args(args) => &args[0],
    };
    let mut child = process
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
        .with_context(|| format!("starting `{program}` in {}", dir.display()))?;

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
            Some(code) => bail!("`{}` exited with code {code}", command.display()),
            None => bail!("`{}` was killed by a signal", command.display()),
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

#[cfg(test)]
#[path = "build_test.rs"]
mod tests;
