//! Small helpers shared by the single-app and monorepo deploy paths.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};

use crate::deploy::report::Reporter;
use crate::remote::Server;

/// Switches one shared release, then installs each process in that release.
pub(super) async fn switch_processes(
    servers: &[&Server],
    id: &str,
    report: &Reporter,
) -> anyhow::Result<()> {
    let first = servers
        .first()
        .ok_or_else(|| anyhow!("no processes to switch"))?;
    if servers.iter().any(|server| {
        server.dir() != first.dir() || !std::ptr::eq(server.session(), first.session())
    }) {
        bail!("processes must share one server and deployment folder");
    }
    first.switch(id, report).await?;
    for server in servers {
        server.install_unit(report).await?;
    }
    Ok(())
}

/// Restarts each process after the shared release has been switched.
pub(super) async fn restart_processes(
    servers: &[&Server],
    report: &Reporter,
) -> anyhow::Result<()> {
    if servers.is_empty() {
        bail!("no processes to restart");
    }
    for server in servers {
        server.restart(report).await?;
    }
    Ok(())
}

/// `1 command`, `3 commands`.
pub(super) fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// Up to five names, and how many more.
pub(super) fn some(names: &[String]) -> String {
    match names.len() {
        0 => "no file named".to_string(),
        n if n <= 5 => names.join(", "),
        n => format!("{} and {} more", names[..5].join(", "), n - 5),
    }
}

/// The first line of an error, short enough for a step's detail column.
pub(super) fn short(err: &anyhow::Error) -> String {
    let text = err.to_string();
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 70 {
        format!("{}…", line.chars().take(69).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Lets a progress update through at most every 1% or 100 ms, and always at
/// the end.
#[derive(Default)]
pub(super) struct Throttle {
    last_percent: Option<u64>,
    last_at: Option<Instant>,
}

impl Throttle {
    pub(super) fn ready(&mut self, done: u64, total: u64) -> bool {
        let percent = (done * 100).checked_div(total).unwrap_or(100);
        let now = Instant::now();
        let due = self
            .last_at
            .is_none_or(|at| now.duration_since(at) >= Duration::from_millis(100));
        if done >= total || self.last_percent != Some(percent) || due {
            self.last_percent = Some(percent);
            self.last_at = Some(now);
            true
        } else {
            false
        }
    }
}
