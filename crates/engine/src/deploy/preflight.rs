//! Preflight: what the server and the local machine need before a deploy starts.

use std::path::Path;

use anyhow::{Result, anyhow, bail};

use super::count;
use super::report::{LogKind, Reporter, Unit};
use crate::build;
use crate::bundle;
use crate::bundle::files::{self, Kind, Source};
use crate::config::{Deployable, Manager, Program};
use crate::human;
use crate::release::ReleaseName;
use crate::remote::{Facts, PM2_NEWEST_MAJOR, Server, parse_version};
use crate::ssh::{Pool, ShellEnv};

/// Below this much free space on the server, preflight stops.
pub(super) const MIN_FREE: u64 = 100 * 1024 * 1024;
/// What shipdeck itself runs on the server, with the package each comes
/// in: the bundle is unpacked with `tar -I zstd` and checked with
/// `sha256sum`.
pub(super) const SERVER_TOOLS: [(&str, &str); 3] =
    [("tar", "tar"), ("zstd", "zstd"), ("sha256sum", "coreutils")];

pub(super) async fn preflight(
    deployable: &Deployable,
    pool: &Pool,
    report: &Reporter,
    step: usize,
) -> Result<(Server, Facts, String)> {
    let config = &deployable.config;
    check_local(deployable)?;
    let endpoint = &deployable.endpoint;
    let address = if endpoint.is_alias() {
        format!(" → {}", endpoint.address())
    } else {
        String::new()
    };
    report.log(
        LogKind::Command,
        format!(
            "connect {}{address} ({})",
            endpoint.label(),
            pool.login().describe()
        ),
    );
    let session = pool.get(endpoint).await?;
    match &session.shell_env {
        ShellEnv::Loaded => report.log(
            LogKind::Info,
            "server commands get the login shell's environment (ssh.shell_env)",
        ),
        ShellEnv::Off => report.log(
            LogKind::Info,
            "server commands run without the login shell's startup files",
        ),
        ShellEnv::Failed(why) => report.log(
            LogKind::Warn,
            format!("{why}; server commands run without them"),
        ),
    }
    let server = Server::new(session, &config.service);
    let mut tools = deployable.required_tools();
    for (tool, _) in SERVER_TOOLS {
        if !tools.iter().any(|t| t == tool) {
            tools.push(tool.into());
        }
    }
    let facts = server.facts(&tools).await?;
    report.progress(step, 1, 2, Unit::Items, "server checked");

    let glibc = facts
        .glibc
        .as_ref()
        .map(|v| format!("glibc {v}"))
        .unwrap_or_else(|| "no glibc".into());
    let free = facts
        .free_bytes
        .map(human::bytes)
        .unwrap_or_else(|| "unknown".into());
    report.log(
        LogKind::Info,
        format!("server: {} · {glibc} · {free} free", facts.arch),
    );
    report.log(
        LogKind::Info,
        format!(
            "live: {} · service {}",
            facts.current.as_deref().unwrap_or("nothing deployed yet"),
            if facts.active.is_empty() {
                "unknown"
            } else {
                &facts.active
            }
        ),
    );

    let dir = &config.service.dir;
    let user = &endpoint.user;
    if !facts.dir_exists {
        // An app's first deploy makes its folder.
        report.log(
            LogKind::Info,
            format!("first deploy of {}: creating {dir}", config.service.name),
        );
        server.create_app_dir(report).await?;
    } else if !facts.dir_writable {
        bail!("{user} cannot write to {dir}");
    }
    if !facts.sudo_missing.is_empty() {
        for command in &facts.sudo_missing {
            report.log(LogKind::Warn, format!("{user} may not run: sudo {command}"));
        }
        bail!(
            "{user} may not run {} of the commands shipdeck needs as root (sudo, without a password)",
            facts.sudo_missing.len()
        );
    }
    check_tools(deployable, &facts, report)?;
    if config.service.manager == Manager::Pm2 {
        check_pm2(deployable, &facts, report)?;
    }
    if let Some(free) = facts.free_bytes
        && free < MIN_FREE
    {
        bail!("only {} free on the server", human::bytes(free));
    }
    let summary = format!("{} · {glibc} · {free} free", facts.arch);
    Ok((server, facts, summary))
}

/// Every program the deploy needs is on the server, and the one the
/// service starts is somewhere the service can reach.
pub(super) fn check_tools(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<()> {
    let mut ours = Vec::new();
    let mut missing = Vec::new();
    for (tool, path) in &facts.tools {
        let package = SERVER_TOOLS
            .iter()
            .find(|(name, _)| name == tool)
            .map(|(_, package)| *package);
        match (path, package) {
            (Some(path), _) => report.log(LogKind::Success, format!("{tool}: {path}")),
            (None, Some(package)) => ours.push((tool.as_str(), package)),
            (None, None) => missing.push(tool.as_str()),
        }
    }
    if !ours.is_empty() {
        let tools: Vec<&str> = ours.iter().map(|(tool, _)| *tool).collect();
        let packages = ours
            .iter()
            .map(|(_, package)| *package)
            .collect::<Vec<_>>()
            .join(" ");
        bail!(
            "the server has no {}, which shipdeck unpacks and checks releases with; install {} \
             (apt install {packages}, or dnf install {packages})",
            tools.join(" or "),
            if ours.len() == 1 { "it" } else { "them" }
        );
    }
    if !missing.is_empty() {
        let path = if deployable.endpoint.shell_env {
            "the PATH the login shell's startup files set"
        } else {
            "the PATH without the login shell's startup files (ssh.shell_env is false)"
        };
        bail!(
            "{} cannot find {} on the server in {path}; install {}, or fix service.start and pipeline.preflight.requires",
            deployable.endpoint.user,
            missing.join(", "),
            if missing.len() == 1 { "it" } else { "them" }
        );
    }
    if let (Program::Tool(tool), _) = deployable.start()
        && let Some(path) = facts.tool_path(&tool)
        && deployable.config.service.manager == Manager::Systemd
        && !deployable.unit_template_path().is_file()
        && (path.starts_with("/home/") || path.starts_with("/root/"))
    {
        bail!(
            "{tool} is at {path}, but the built-in unit hides home folders from the service \
             (ProtectHome=true). Install {tool} system-wide (e.g. /usr/bin or /usr/local/bin), \
             or add a service.template without ProtectHome"
        );
    }
    Ok(())
}

/// PM2 is there, with a warning when it is newer than shipdeck is tested
/// with, and no systemd unit of the same name still runs the app.
pub(super) fn check_pm2(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<()> {
    let output = facts.pm2_version.as_deref().unwrap_or_default();
    let (version, major) = parse_version(output).map_err(|why| anyhow!(why))?;
    if major > PM2_NEWEST_MAJOR {
        report.log(
            LogKind::Warn,
            format!(
                "pm2 {version} is newer than this shipdeck is tested with \
                 ({PM2_NEWEST_MAJOR}.x); deploying anyway"
            ),
        );
    } else {
        report.log(LogKind::Success, format!("pm2 {version}"));
    }
    let name = &deployable.config.service.name;
    if matches!(
        facts.systemd.as_str(),
        "active" | "activating" | "reloading"
    ) {
        bail!(
            "the systemd service {name} is still running and would clash with PM2; \
             stop it first: sudo systemctl disable --now {name}"
        );
    }
    Ok(())
}

/// Where preflight found the start program, when it is looked up by name.
pub(super) fn start_path(deployable: &Deployable, facts: &Facts) -> Option<String> {
    match deployable.start() {
        (Program::Tool(tool), _) => facts.tool_path(&tool).map(str::to_string),
        _ => None,
    }
}

/// What can be checked on this machine before connecting: the project
/// folder and the service file.
pub(super) fn check_local(deployable: &Deployable) -> Result<()> {
    let project = deployable.project_dir();
    if !project.is_dir() {
        bail!(
            "the project folder {} does not exist (project: {})",
            project.display(),
            deployable.config.project
        );
    }
    let template = deployable.unit_template_path();
    if deployable.config.service.manager == Manager::Pm2 && template.is_file() {
        bail!(
            "{} is a systemd unit, but service.manager is pm2; remove it, or use systemd",
            template.display()
        );
    }
    let sample = ReleaseName::new("0.0.0", "check", false, chrono::Utc::now());
    bundle::service_file(deployable, &sample, None)?;
    Ok(())
}

/// Checks what the build produced: the binary can run on the server, and
/// every project pattern in `pipeline.pack.files` finds something.
pub(super) fn check_build(
    deployable: &Deployable,
    facts: &Facts,
    report: &Reporter,
) -> Result<String> {
    let mut summary = Vec::new();
    if let Some(binary) = deployable.binary_path() {
        summary.push(check_binary(&binary, facts, report)?);
    }
    let items = files::walk_source(deployable, Source::Project)?;
    if !items.is_empty() {
        let count_files = items
            .values()
            .filter(|i| matches!(i.kind, Kind::File { .. }))
            .count();
        report.log(
            LogKind::Success,
            format!(
                "project files: {} from {}",
                count(count_files, "file"),
                deployable
                    .config
                    .pipeline
                    .pack
                    .files
                    .include
                    .project
                    .join(", ")
            ),
        );
        summary.push(count(count_files, "file"));
    }
    Ok(summary.join(" · "))
}

/// Checks the built binary can run on the server; returns a short summary.
pub(super) fn check_binary(binary: &Path, facts: &Facts, report: &Reporter) -> Result<String> {
    let info = build::inspect(binary)?;
    if info.arch != facts.arch {
        bail!(
            "the binary is built for {}, but the server is {}",
            info.arch,
            facts.arch
        );
    }
    let size = human::bytes(info.size);
    let Some(needed) = info.glibc else {
        report.log(LogKind::Info, format!("binary: {size}, no glibc needed"));
        return Ok(format!("{size} · static"));
    };
    report.log(
        LogKind::Info,
        format!("binary: {size}, needs glibc {needed} or newer"),
    );
    match &facts.glibc {
        None => bail!(
            "the binary needs glibc {needed}, but the server has no glibc; build a static musl binary instead"
        ),
        Some(have) if *have < needed => bail!(
            "the binary needs glibc {needed}, but the server has {have}. \
             Build against the server's glibc, e.g. cargo zigbuild --target {}-unknown-linux-gnu.{have}",
            info.arch
        ),
        Some(_) => Ok(format!("{size} · glibc ≥ {needed}")),
    }
}
