//! PM2 on the server: which versions shipdeck runs with, and the app's state
//! read from `pm2 jlist`.

use anyhow::Result;
use serde_json::Value;

use super::Server;

/// The newest PM2 major version shipdeck is tested with. A newer one is
/// refused at preflight rather than trusted to behave the same.
pub const PM2_NEWEST_MAJOR: u64 = 6;

/// Checks the output of `pm2 -v` against [`PM2_NEWEST_MAJOR`].
pub fn check_version(output: &str) -> Result<String, String> {
    let Some(version) = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with(|c: char| c.is_ascii_digit()))
    else {
        return Err(format!(
            "could not read PM2's version (`pm2 -v` said {:?})",
            output.trim()
        ));
    };
    let major = version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u64>().ok());
    match major {
        Some(major) if major <= PM2_NEWEST_MAJOR => Ok(version.to_string()),
        Some(_) => Err(format!(
            "the server has PM2 {version}, newer than this shipdeck supports \
             (up to {PM2_NEWEST_MAJOR}.x); update shipdeck, or install pm2@{PM2_NEWEST_MAJOR}"
        )),
        None => Err(format!("could not read PM2's version {version:?}")),
    }
}

/// The app's state from `pm2 jlist`, in systemd's words so the screens
/// colour it alike, with its restart count summed over its instances.
/// `missing` when PM2 has no app of that name.
pub fn app_state(jlist: &str, app: &str) -> (String, Option<u64>) {
    // PM2 may print a notice before the JSON.
    let json = jlist.find('[').map(|at| &jlist[at..]).unwrap_or("");
    let Ok(Value::Array(processes)) = serde_json::from_str::<Value>(json.trim_end()) else {
        return (String::new(), None);
    };
    let mine: Vec<&Value> = processes
        .iter()
        .filter(|p| p.get("name").and_then(Value::as_str) == Some(app))
        .collect();
    if mine.is_empty() {
        return ("missing".into(), None);
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
    (state.into(), restarts)
}

impl Server {
    /// The app's state and restart count, or nothing when PM2 cannot be
    /// asked.
    pub(super) async fn pm2_state(&self) -> Result<(String, Option<u64>)> {
        let out = self
            .session
            .exec("command -v pm2 >/dev/null 2>&1 && pm2 jlist 2>/dev/null")
            .await?;
        if !out.success() {
            return Ok((String::new(), None));
        }
        Ok(app_state(&out.stdout, &self.pm2_app))
    }
}

#[cfg(test)]
#[path = "pm2_test.rs"]
mod tests;
