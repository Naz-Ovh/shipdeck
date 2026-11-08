//! The service by hand: its live state, and stop, start and restart.
//!
//! The agent runs the tools directly, without a shell. Systemd commands use
//! `sudo -n` when the SSH user is not root; PM2 runs as that user.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use shipdeck_protocol::message::agent::{
    AgentManager, AgentOk, AgentService, AgentServiceAction, AgentServiceControl, AgentStatus,
};

/// Runs one request outside the asynchronous SSH reader.
pub(crate) async fn status(service: AgentService) -> Result<AgentStatus> {
    tokio::task::spawn_blocking(move || status_sync(&service)).await?
}

/// Runs one service change outside the asynchronous SSH reader.
pub(crate) async fn control(request: AgentServiceControl) -> Result<AgentOk> {
    tokio::task::spawn_blocking(move || control_sync(&request)).await?
}

/// Refuses names that could read as an option or leave the service's unit.
pub(crate) fn safe_service(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._@-".contains(ch))
}

fn status_sync(service: &AgentService) -> Result<AgentStatus> {
    let (dir_exists, current) = deployment(service)?;
    let active = state(service)?;
    Ok(AgentStatus {
        dir_exists,
        current,
        active,
    })
}

fn control_sync(request: &AgentServiceControl) -> Result<AgentOk> {
    let service = &request.service;
    let (dir_exists, _) = deployment(service)?;
    if !dir_exists {
        bail!("the deployment folder does not exist");
    }
    match service.manager {
        AgentManager::Systemd => {
            let output = run_systemctl(request.action, &service.service)?;
            check(
                &output,
                &format!("{} {}", verb(request.action), service.service),
            )?;
        }
        AgentManager::Pm2 => {
            if request.action == AgentServiceAction::Start && state(service)? == "active" {
                return Ok(AgentOk {});
            }
            let verb = match request.action {
                AgentServiceAction::Stop => "stop",
                AgentServiceAction::Start => "start",
                AgentServiceAction::Restart => "startOrReload",
            };
            let file = pm2_file(service)?;
            let mut environment = environment(service)?;
            environment.insert("PM2_HOME".into(), pm2_home()?);
            let output = Command::new("pm2")
                .arg(verb)
                .arg(&file)
                .args(["--only", &service.service, "--update-env"])
                .envs(&environment)
                .output()
                .context("running pm2")?;
            check(&output, &format!("{verb} {}", service.service))?;
            save_pm2(&environment)?;
        }
    }
    Ok(AgentOk {})
}

fn verb(action: AgentServiceAction) -> &'static str {
    match action {
        AgentServiceAction::Stop => "stop",
        AgentServiceAction::Start => "start",
        AgentServiceAction::Restart => "restart",
    }
}

/// The app folder, whether `current` exists, and the release it names.
fn deployment(service: &AgentService) -> Result<(bool, Option<String>)> {
    let root = checked_root(&service.root)?;
    let dir_exists = root.is_dir();
    let current = match std::fs::read_link(root.join("current")) {
        Ok(path) => Some(
            path.file_name()
                .and_then(|name| name.to_str())
                .context("the current release name is not UTF-8")?
                .to_string(),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading the current release"),
    };
    Ok((dir_exists, current))
}

fn checked_root(root: &str) -> Result<PathBuf> {
    let path = Path::new(root);
    if !path.is_absolute() {
        bail!("deployment root must be absolute");
    }
    Ok(path.to_path_buf())
}

fn state(service: &AgentService) -> Result<String> {
    if !safe_service(&service.service) {
        bail!("invalid service name");
    }
    match service.manager {
        AgentManager::Systemd => {
            let output = systemctl(&["is-active", &service.service])?;
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(if text.is_empty() {
                "unknown".to_string()
            } else {
                text
            })
        }
        AgentManager::Pm2 => {
            let output = Command::new("pm2")
                .arg("jlist")
                .output()
                .context("running pm2 jlist")?;
            if output.status.code() == Some(127) {
                return Ok(String::new());
            }
            check(&output, "pm2 jlist")?;
            let text = String::from_utf8_lossy(&output.stdout);
            pm2_state(&text, &service.service)
        }
    }
}

/// Maps PM2's status words to systemd's, as the engine's own PM2 reader does.
fn pm2_state(jlist: &str, app: &str) -> Result<String> {
    let processes: Vec<Value> = serde_json::from_str(jlist.trim()).map_err(|err| {
        let first = jlist.lines().next().unwrap_or_default();
        anyhow::anyhow!("`pm2 jlist` did not print JSON ({err}); it said: {first}")
    })?;
    let statuses: Vec<&str> = processes
        .iter()
        .filter(|process| process.get("name").and_then(Value::as_str) == Some(app))
        .map(|process| {
            process
                .pointer("/pm2_env/status")
                .and_then(Value::as_str)
                .unwrap_or("")
        })
        .collect();
    if statuses.is_empty() {
        return Ok("missing".to_string());
    }
    Ok(if statuses.contains(&"errored") {
        "failed"
    } else if statuses.iter().all(|state| *state == "online") {
        "active"
    } else if statuses.contains(&"stopping") {
        "deactivating"
    } else if statuses.iter().all(|state| *state == "stopped") {
        "inactive"
    } else {
        "activating"
    }
    .to_string())
}

fn systemctl(args: &[&str]) -> Result<Output> {
    let mut command;
    if is_root() {
        command = Command::new("systemctl");
    } else {
        command = Command::new("sudo");
        command.args(["-n", "systemctl"]);
    }
    command.args(args).output().context("running systemctl")
}

fn run_systemctl(action: AgentServiceAction, service: &str) -> Result<Output> {
    systemctl(&[verb(action), service])
}

fn check(output: &Output, what: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&output.stderr).trim().to_string();
    bail!(
        "{what} failed: {}",
        if why.is_empty() {
            format!("exit {}", output.status)
        } else {
            why
        }
    )
}

/// The PM2 ecosystem file the live release carries.
fn pm2_file(service: &AgentService) -> Result<PathBuf> {
    let relative = Path::new(&service.pm2_file);
    if service.pm2_file.is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        bail!("invalid PM2 file path");
    }
    let file = checked_root(&service.root)?.join("current").join(relative);
    if !file.is_file() {
        bail!(
            "this release has no {}; was it deployed with systemd?",
            service.pm2_file
        );
    }
    Ok(file)
}

/// `service.env`, then the release's env file, as systemd reads it.
fn environment(service: &AgentService) -> Result<BTreeMap<String, String>> {
    let mut environment = service.env.clone();
    if let Some(file) = &service.env_file {
        if file.is_empty()
            || !Path::new(file)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            bail!("invalid environment file path");
        }
        let path = checked_root(&service.root)?.join("current").join(file);
        if let Ok(text) = std::fs::read(&path) {
            environment.extend(crate::env_file::parse(&text));
        }
    }
    Ok(environment)
}

fn pm2_home() -> Result<String> {
    if let Some(home) = std::env::var_os("PM2_HOME") {
        return Ok(home.to_string_lossy().into_owned());
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(Path::new(&home).join(".pm2").to_string_lossy().into_owned())
}

fn save_pm2(environment: &BTreeMap<String, String>) -> Result<()> {
    let output = Command::new("pm2")
        .arg("save")
        .envs(environment)
        .output()
        .context("running pm2 save")?;
    check(&output, "pm2 save")
}

pub(crate) fn is_root() -> bool {
    static ROOT: OnceLock<bool> = OnceLock::new();
    *ROOT.get_or_init(|| {
        Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .is_some_and(|text| text.trim() == "0")
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
