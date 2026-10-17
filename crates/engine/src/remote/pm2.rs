//! PM2 on the server: which versions shipdeck runs with, and the app's state
//! read from `pm2 jlist`.

use anyhow::Result;
use serde_json::Value;

use super::Server;

/// The newest PM2 major version shipdeck is tested with. shipdeck uses
/// only PM2's basic commands, so a newer one still deploys; preflight
/// warns that it is untested.
pub const PM2_NEWEST_MAJOR: u64 = 7;

/// Reads the output of `pm2 -v`, which is the version alone, as the
/// version and its major number.
pub fn parse_version(output: &str) -> Result<(String, u64), String> {
    let version = output.trim();
    let parts: Option<Vec<u64>> = version.split('.').map(|part| part.parse().ok()).collect();
    let Some([major, _, _]) = parts.as_deref() else {
        return Err(format!("`pm2 -v` did not print a version: {version:?}"));
    };
    Ok((version.to_string(), *major))
}

/// The app's state from `pm2 jlist`, which prints the process list as
/// JSON, in systemd's words so the screens colour it alike, with its
/// restart count summed over its instances. `missing` when PM2 has no app
/// of that name.
pub fn app_state(jlist: &str, app: &str) -> Result<(String, Option<u64>), String> {
    let processes: Vec<Value> = serde_json::from_str(jlist.trim()).map_err(|err| {
        let first = jlist.lines().next().unwrap_or_default();
        format!("`pm2 jlist` did not print JSON ({err}); it said: {first}")
    })?;
    let mine: Vec<&Value> = processes
        .iter()
        .filter(|p| p.get("name").and_then(Value::as_str) == Some(app))
        .collect();
    if mine.is_empty() {
        return Ok(("missing".into(), None));
    }
    let statuses: Vec<&str> = mine
        .iter()
        .map(|p| {
            p.pointer("/pm2_env/status")
                .and_then(Value::as_str)
                .unwrap_or("")
        })
        .collect();
    let restarts = mine
        .iter()
        .map(|p| p.pointer("/pm2_env/restart_time").and_then(Value::as_u64))
        .sum::<Option<u64>>();
    let state = if statuses.contains(&"errored") {
        "failed"
    } else if statuses.iter().all(|s| *s == "online") {
        "active"
    } else if statuses.contains(&"stopping") {
        "deactivating"
    } else if statuses.iter().all(|s| *s == "stopped") {
        "inactive"
    } else {
        "activating"
    };
    Ok((state.into(), restarts))
}

impl Server {
    /// The app's state and restart count; nothing when the server has no
    /// `pm2` (preflight says so).
    pub(super) async fn pm2_state(&self) -> Result<(String, Option<u64>)> {
        let out = self
            .session
            .exec("command -v pm2 >/dev/null 2>&1 || exit 127\npm2 jlist")
            .await?;
        if out.code == 127 {
            return Ok((String::new(), None));
        }
        let out = out.check("pm2 jlist")?;
        app_state(&out.stdout, &self.pm2_app).map_err(anyhow::Error::msg)
    }
}

#[cfg(test)]
#[path = "pm2_test.rs"]
mod tests;
