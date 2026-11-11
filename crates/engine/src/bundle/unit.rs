//! The systemd unit each release carries in `.shipdeck/`, rendered from the
//! built-in template or the app's `process.systemd.template`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Context, Result, bail};

use crate::config::placeholders::{self, AppValues, Values};
use crate::config::{Deployment, Program, SystemdOptions};
use crate::release::ReleaseName;

/// The built-in unit, before the placeholders and the app's own values.
const DEFAULT_TEMPLATE: &str = r"# {{PROCESS}}.service for release {{RELEASE_ID}}, written by shipdeck.
# Every deploy and rollback replaces this file; change process.systemd.template instead.

[Unit]
Description={{NAME}} {{APP}} ({{VERSION}})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={{APP_CURRENT}}
ExecStart={exec}
Environment=SHIPDECK_RELEASE={{RELEASE_FOLDER}}
{env}
Restart=always
RestartSec=2
KillSignal=SIGTERM
TimeoutStopSec=30
LimitNOFILE=65535

NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths={{SHARED}}
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
LockPersonality=true

[Install]
WantedBy=multi-user.target
";

/// The app's unit file in the release.
#[must_use]
pub fn unit_file(deployment: &Deployment, app: &str) -> String {
    format!(".shipdeck/{}.service", deployment.process_name(app))
}

/// Renders the app's unit, or `None` when it does not use systemd.
///
/// # Errors
/// Fails when the custom template cannot be read, a placeholder is unknown,
/// or the app cannot start.
pub fn render(
    deployment: &Deployment,
    app: &str,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<Option<String>> {
    let systemd = deployment
        .systemd(app)
        .context("app has no systemd settings")?;
    let id = release.id();
    let values = values(deployment, app, &id, &release.version);
    let text = match deployment.unit_template(app).filter(|path| path.is_file()) {
        Some(path) => {
            let template = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            placeholders::expand(&template, &values)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("rendering {}", path.display()))?
        }
        None => built_in(deployment, app, start_path, &values)?,
    };
    let text = match &systemd.options {
        Some(options) => apply_options(&text, options),
        None => text,
    };
    Ok(Some(text))
}

/// The placeholders' values for one app of one release.
fn values<'a>(
    deployment: &'a Deployment,
    app: &'a str,
    id: &'a str,
    version: &'a str,
) -> Values<'a> {
    Values {
        root: deployment.dir(),
        name: deployment.name(),
        host: &deployment.endpoint.name,
        project: deployment.project.to_str().unwrap_or("."),
        config: deployment.config_dir.to_str().unwrap_or("."),
        release: Some(id),
        version: Some(version),
        app: Some(AppValues {
            name: app,
            source: deployment.source(app).display().to_string(),
            process: deployment
                .process(app)
                .map(|_| deployment.process_name(app)),
        }),
    }
}

/// The built-in unit with its values filled in.
fn built_in(
    deployment: &Deployment,
    app: &str,
    start_path: Option<&str>,
    values: &Values<'_>,
) -> Result<String> {
    let current = format!("{}/current/apps/{app}", deployment.dir());
    let exec = exec_start(deployment, app, start_path, &current)?;
    let release_folder = format!(
        "{}/releases/{}",
        deployment.dir(),
        values.release.unwrap_or("")
    );
    let mut env = String::new();
    if let Some(file) = deployment.env_file(app) {
        let _ = writeln!(env, "EnvironmentFile={current}/{file}");
    }
    for (key, value) in deployment.app_env(app) {
        let value = placeholders::expand(&value, values).map_err(anyhow::Error::msg)?;
        let _ = writeln!(env, "Environment=\"{key}={value}\"");
    }
    let template = DEFAULT_TEMPLATE
        .replace("{user}", &deployment.run_user(app))
        .replace("{exec}", &exec)
        .replace("{env}", env.trim_end())
        .replace("{RELEASE_FOLDER}", &release_folder);
    placeholders::expand(&template, values).map_err(|why| anyhow::anyhow!("{why}"))
}

/// `ExecStart=`: the program, with a program inside the release made
/// absolute through `current`.
fn exec_start(
    deployment: &Deployment,
    app: &str,
    start_path: Option<&str>,
    current: &str,
) -> Result<String> {
    let Some((program, rest)) = deployment.start(app) else {
        bail!("app {app} has no process.start and no binary");
    };
    let program = match program {
        Program::Absolute(path) => path,
        Program::InRelease(path) => format!("{current}/{path}"),
        // systemd finds a bare name on its own fixed PATH, which may not be
        // where preflight found it; the found path is the one to use.
        Program::Tool(name) => start_path.map_or(name, str::to_string),
    };
    Ok(if rest.is_empty() {
        program
    } else {
        format!("{program} {rest}")
    })
}

/// Puts the app's own properties into the unit: a property replaces the
/// built-in line of the same name, or is added; a list writes one line per
/// item.
fn apply_options(text: &str, options: &SystemdOptions) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (section, entries) in [
        ("Unit", &options.unit),
        ("Service", &options.service),
        ("Install", &options.install),
    ] {
        if entries.is_empty() {
            continue;
        }
        let Some(start) = lines
            .iter()
            .position(|line| line.trim() == format!("[{section}]"))
        else {
            if !lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.extend(option_lines(entries));
            continue;
        };
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.trim_start().starts_with('['))
            .map_or(lines.len(), |offset| start + 1 + offset);
        let mut kept: Vec<String> = lines[start + 1..end]
            .iter()
            .filter(|line| {
                let key = line.split('=').next().unwrap_or_default().trim();
                !entries.keys().any(|own| own == key)
            })
            .cloned()
            .collect();
        kept.extend(option_lines(entries));
        lines.splice(start + 1..end, kept);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// One line per value, or per item of a list.
fn option_lines(entries: &BTreeMap<String, serde_json::Value>) -> Vec<String> {
    let mut lines = Vec::new();
    for (key, value) in entries {
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    lines.push(format!("{key}={}", scalar(item)));
                }
            }
            other => lines.push(format!("{key}={}", scalar(other))),
        }
    }
    lines
}

fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}
