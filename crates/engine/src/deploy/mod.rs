//! The deploy: preflight, build, pack, upload, switch, restart, health,
//! rotate. A failure after the switch puts the previous release back.

pub mod report;
mod rollback;

pub use rollback::{ROLLBACK_STEPS, rollback};

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use crate::build;
use crate::bundle::{self, unit};
use crate::config::{CommandLine, DeployConfig, Deployable, HealthConfig, Program};
use crate::human;
use crate::release::Manifest;
use crate::remote::{Facts, HttpCheck, Server, plan_rotation};
use crate::setup;
use crate::ssh::{Login, Pool};
use report::{LogKind, Outcome, Reporter, Unit};

/// Every step a deploy can have, in order. Hook steps appear only when
/// `deploy.yaml` has hooks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Before,
    Preflight,
    Build,
    Pack,
    Upload,
    Prepare,
    Switch,
    Restart,
    Health,
    Rotate,
    After,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Self::Before => "Before",
            Self::Preflight => "Preflight",
            Self::Build => "Build",
            Self::Pack => "Pack",
            Self::Upload => "Upload",
            Self::Prepare => "Prepare",
            Self::Switch => "Switch",
            Self::Restart => "Restart",
            Self::Health => "Health",
            Self::Rotate => "Rotate",
            Self::After => "After",
        }
    }
}

/// The steps a deploy of `config` goes through.
pub fn deploy_steps(config: &DeployConfig) -> Vec<Step> {
    let hooks = &config.hooks;
    [
        (Step::Before, !hooks.before.is_empty()),
        (Step::Preflight, true),
        (Step::Build, true),
        (Step::Pack, true),
        (Step::Upload, true),
        (Step::Prepare, !hooks.server.is_empty()),
        (Step::Switch, true),
        (Step::Restart, true),
        (Step::Health, true),
        (Step::Rotate, true),
        (Step::After, !hooks.after.is_empty()),
    ]
    .into_iter()
    .filter_map(|(step, used)| used.then_some(step))
    .collect()
}

/// The names of [`deploy_steps`], for the screen.
pub fn deploy_step_names(config: &DeployConfig) -> Vec<&'static str> {
    deploy_steps(config).into_iter().map(Step::name).collect()
}

/// Step indices for one deploy, as the reporter counts them.
struct Plan(Vec<Step>);

impl Plan {
    fn at(&self, step: Step) -> usize {
        self.0
            .iter()
            .position(|s| *s == step)
            .expect("the plan has every step the deploy reports")
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Below this much free space on the server, preflight stops.
const MIN_FREE: u64 = 100 * 1024 * 1024;
/// Consecutive healthy probes, one second apart, before a release counts as up.
const STABLE_PROBES: u32 = 3;

pub async fn deploy(deployable: Deployable, pool: Arc<Pool>, report: Reporter) {
    let outcome = run(&deployable, &pool, &report).await;
    report.done(outcome);
}

async fn run(deployable: &Deployable, pool: &Pool, report: &Reporter) -> Outcome {
    let config = &deployable.config;
    let plan = Plan(deploy_steps(config));
    let total = plan.len();
    let stop = |step: Step, err: anyhow::Error| stop(report, plan.at(step), total, err);

    // Before: the local hooks, ahead of everything else.
    if !config.hooks.before.is_empty() {
        let step = plan.at(Step::Before);
        report.start(step, "Before: running hooks.before on this machine");
        let started = Instant::now();
        if let Err(err) = local_hooks(deployable, &config.hooks.before, None, report, step).await {
            return stop(Step::Before, err);
        }
        report.finish(
            step,
            format!(
                "{} · {}",
                count(config.hooks.before.len(), "command"),
                human::duration(started.elapsed())
            ),
        );
    }

    // Preflight
    let step = plan.at(Step::Preflight);
    report.start(step, "Preflight: checking the server");
    let (server, facts) = match preflight(deployable, pool, report, step).await {
        Ok(found) => found,
        Err(err) => return stop(Step::Preflight, err),
    };

    // Build
    let step = plan.at(Step::Build);
    report.start(step, "Build: building on this machine");
    let (name, source) = match build::release_name(deployable).await {
        Ok(found) => found,
        Err(err) => return stop(Step::Build, err),
    };
    let id = name.id();
    report.release(&id);
    report.log(
        LogKind::Info,
        format!("release {id} (version from {source})"),
    );
    if name.dirty {
        report.log(
            LogKind::Warn,
            "the source has uncommitted changes; the release is marked dirty",
        );
    }
    let started = Instant::now();
    if let Err(err) = build::run(deployable, report, step).await {
        return stop(Step::Build, err);
    }
    let built = match check_build(deployable, &facts, report) {
        Ok(summary) => summary,
        Err(err) => return stop(Step::Build, err),
    };
    report.finish(
        step,
        format!("{built} · {}", human::duration(started.elapsed())),
    );

    // Pack
    let step = plan.at(Step::Pack);
    report.start(step, "Pack: bundling the release");
    let deployer = build::deployer(&deployable.build_dir()).await;
    let manifest = Manifest::new(&name, &deployer);
    let packed = async {
        let start_path = start_path(deployable, &facts);
        let unit = unit::render_for(deployable, &name, start_path.as_deref())?;
        report.log(
            LogKind::Info,
            format!(
                "ExecStart={}",
                unit::exec_start(deployable, start_path.as_deref())
            ),
        );
        let (owned, reporter) = (deployable.clone(), report.clone());
        let mut throttle = Throttle::default();
        let bundle = tokio::task::spawn_blocking(move || {
            bundle::pack(&owned, &manifest, &unit, |done, total| {
                if throttle.ready(done, total) {
                    reporter.progress(step, done, total, Unit::Bytes, "compressing");
                }
            })
        })
        .await
        .map_err(|err| anyhow!("packing stopped: {err}"))??;
        if let Some(free) = facts.free_bytes {
            let needed = bundle.data.len() as u64 * 2 + bundle.input_bytes;
            if free < needed {
                bail!(
                    "the server has {} free; this release needs about {}",
                    human::bytes(free),
                    human::bytes(needed)
                );
            }
        }
        Ok(bundle)
    }
    .await;
    let bundle = match packed {
        Ok(bundle) => bundle,
        Err(err) => return stop(Step::Pack, err),
    };
    report.finish(
        step,
        format!(
            "{} files · {} → {}",
            bundle.files,
            human::bytes(bundle.input_bytes),
            human::bytes(bundle.data.len() as u64)
        ),
    );

    // Upload and unpack
    let step = plan.at(Step::Upload);
    report.start(step, "Upload: sending the release");
    let started = Instant::now();
    let uploaded = async {
        server.prepare(report).await?;
        let target = server.upload_path(&id);
        report.log(
            LogKind::Command,
            format!(
                "{} › upload {} to {target}",
                server.session().label,
                human::bytes(bundle.data.len() as u64)
            ),
        );
        let mut throttle = Throttle::default();
        server
            .session()
            .upload(&bundle.data, &target, |sent, total| {
                if throttle.ready(sent, total) {
                    let speed = sent as f64 / started.elapsed().as_secs_f64().max(0.001);
                    report.progress(
                        step,
                        sent,
                        total,
                        Unit::Bytes,
                        format!("{}/s", human::bytes(speed as u64)),
                    );
                }
            })
            .await?;
        server.unpack(&id, report).await
    }
    .await;
    if let Err(err) = uploaded {
        let _ = server.discard(&id, report).await;
        return stop(Step::Upload, err);
    }
    report.finish(
        step,
        format!(
            "{} in {}",
            human::bytes(bundle.data.len() as u64),
            human::duration(started.elapsed())
        ),
    );

    // Prepare: the server hooks, inside the new release, before it goes live.
    if !config.hooks.server.is_empty() {
        let step = plan.at(Step::Prepare);
        report.start(
            step,
            &format!("Prepare: running hooks.server in releases/{id}"),
        );
        let started = Instant::now();
        let hooks = &config.hooks.server;
        let mut prepared = Ok(());
        for (done, hook) in hooks.iter().enumerate() {
            report.progress(
                step,
                done as u64,
                hooks.len() as u64,
                Unit::Items,
                hook.trim(),
            );
            prepared = server.hook(&id, hook, report).await;
            if prepared.is_err() {
                break;
            }
        }
        if let Err(err) = prepared {
            // Nothing live has changed yet.
            let _ = server.discard(&id, report).await;
            return stop(Step::Prepare, err);
        }
        report.finish(
            step,
            format!(
                "{} · {}",
                count(hooks.len(), "command"),
                human::duration(started.elapsed())
            ),
        );
    }

    // Switch
    let step = plan.at(Step::Switch);
    report.start(step, "Switch: making it the current release");
    let previous = facts.current.clone();
    let switched = async {
        server.switch(&id, report).await?;
        server.install_unit(report).await
    }
    .await;
    if let Err(err) = switched {
        report.fail(step, short(&err));
        report.log(LogKind::Error, format!("{err:#}"));
        // Nothing was restarted, so the old process is still serving.
        return match &previous {
            Some(previous) => match restore(&server, previous, report).await {
                Ok(()) => {
                    let _ = server.discard(&id, report).await;
                    skip_rest(report, step, total, "stopped");
                    Outcome::Failed {
                        error: format!("{err:#}"),
                    }
                }
                Err(restore_err) => Outcome::RollbackFailed {
                    error: format!("{err:#}"),
                    rollback_error: format!("{restore_err:#}"),
                },
            },
            None => {
                skip_rest(report, step, total, "stopped");
                Outcome::Failed {
                    error: format!("{err:#}"),
                }
            }
        };
    }
    report.finish(step, format!("current → {id}"));

    // Restart
    let step = plan.at(Step::Restart);
    report.start(step, &format!("Restart: {}", config.remote.service));
    if let Err(err) = server.restart(report).await {
        return recover(
            &server,
            report,
            step,
            err,
            previous.as_deref(),
            Some(&id),
            total,
        )
        .await;
    }
    report.finish(step, "");

    // Health
    let step = plan.at(Step::Health);
    report.start(step, "Health: waiting for the new release to settle");
    match wait_healthy(&server, &config.health, report, step).await {
        Ok(summary) => report.finish(step, summary),
        Err(err) => {
            return recover(
                &server,
                report,
                step,
                err,
                previous.as_deref(),
                Some(&id),
                total,
            )
            .await;
        }
    }

    // Rotate
    let step = plan.at(Step::Rotate);
    report.start(step, "Rotate: tidying old releases");
    match rotate(&server, deployable, report, step).await {
        Ok(summary) => report.finish(step, summary),
        Err(err) => {
            report.log(LogKind::Warn, format!("rotation: {err:#}"));
            report.warn(step, short(&err));
        }
    }
    report.log(LogKind::Success, format!("{id} is live"));

    // After: the local hooks, once it is live. A failure changes nothing.
    if !config.hooks.after.is_empty() {
        let step = plan.at(Step::After);
        report.start(step, "After: running hooks.after on this machine");
        let started = Instant::now();
        match local_hooks(deployable, &config.hooks.after, Some(&id), report, step).await {
            Ok(()) => report.finish(
                step,
                format!(
                    "{} · {}",
                    count(config.hooks.after.len(), "command"),
                    human::duration(started.elapsed())
                ),
            ),
            Err(err) => {
                report.log(
                    LogKind::Warn,
                    format!("hooks.after: {err:#}; {id} stays live"),
                );
                report.warn(step, short(&err));
            }
        }
    }

    Outcome::Succeeded { release: id }
}

/// Runs `hooks` one after another on this machine, in `build.cwd`.
async fn local_hooks(
    deployable: &Deployable,
    hooks: &[String],
    release: Option<&str>,
    report: &Reporter,
    step: usize,
) -> Result<()> {
    let dir = deployable.build_dir();
    if !dir.is_dir() {
        bail!("build.cwd {} does not exist", dir.display());
    }
    let env = build::local_env(deployable, release);
    for (done, hook) in hooks.iter().enumerate() {
        report.progress(
            step,
            done as u64,
            hooks.len() as u64,
            Unit::Items,
            hook.trim(),
        );
        build::run_local(&CommandLine::Shell(hook.clone()), &dir, &env, report, step).await?;
    }
    Ok(())
}

async fn preflight(
    deployable: &Deployable,
    pool: &Pool,
    report: &Reporter,
    step: usize,
) -> Result<(Server, Facts)> {
    let config = &deployable.config;
    check_local(deployable, report)?;
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
    let session = match pool.get(endpoint).await {
        Ok(session) => session,
        Err(err) => {
            setup_hint(
                deployable,
                pool.login(),
                report,
                "if the server is not set up yet",
            );
            return Err(err);
        }
    };
    let server = Server::new(session, &config.remote);
    let tools = deployable.required_tools();
    let facts = server.facts(&endpoint.user, &tools).await?;

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

    let dir = &config.remote.dir;
    let user = &endpoint.user;
    if !facts.dir_exists || !facts.dir_writable {
        setup_hint(
            deployable,
            pool.login(),
            report,
            "the server needs its one-time setup",
        );
        if !facts.dir_exists {
            bail!("{dir} does not exist on the server");
        }
        bail!("{user} cannot write to {dir}");
    }
    if !facts.sudo_missing.is_empty() {
        for command in &facts.sudo_missing {
            report.log(LogKind::Warn, format!("sudo not allowed: {command}"));
        }
        setup_hint(
            deployable,
            pool.login(),
            report,
            "the server needs its one-time setup",
        );
        report.log(
            LogKind::Info,
            format!(
                "or add this line yourself with: sudo visudo -f {}",
                setup::sudoers_file(&config.remote.service)
            ),
        );
        report.log(LogKind::Command, facts.sudoers_line.clone());
        bail!(
            "{user} may not run {} of the commands shipdeck needs without a password",
            facts.sudo_missing.len()
        );
    }
    check_tools(deployable, &facts, report)?;
    if let Some(free) = facts.free_bytes
        && free < MIN_FREE
    {
        bail!("only {} free on the server", human::bytes(free));
    }

    report.finish(step, format!("{} · {glibc} · {free} free", facts.arch));
    Ok((server, facts))
}

/// Every program the deploy needs is on the server, and the one the
/// service starts is somewhere the service can reach.
fn check_tools(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<()> {
    let mut missing = Vec::new();
    for (tool, path) in &facts.tools {
        match path {
            Some(path) => report.log(LogKind::Success, format!("{tool}: {path}")),
            None => missing.push(tool.as_str()),
        }
    }
    if !missing.is_empty() {
        bail!(
            "{} cannot find {} on the server; install {} or fix remote.start and remote.requires",
            deployable.endpoint.user,
            missing.join(", "),
            if missing.len() == 1 { "it" } else { "them" }
        );
    }
    if let (Program::Tool(tool), _) = deployable.start()
        && let Some(path) = facts.tool_path(&tool)
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

/// Where preflight found the start program, when it is looked up by name.
fn start_path(deployable: &Deployable, facts: &Facts) -> Option<String> {
    match deployable.start() {
        (Program::Tool(tool), _) => facts.tool_path(&tool).map(str::to_string),
        _ => None,
    }
}

/// Shows the one-time setup command; `u` on the Deployables screen shows it too.
pub(crate) fn setup_hint(deployable: &Deployable, login: &Login, report: &Reporter, why: &str) {
    report.log(
        LogKind::Info,
        format!(
            "{why}: run this once on the server as root (or press u on the Deployables screen):"
        ),
    );
    report.log(LogKind::Command, setup::command(deployable, login).command);
}

/// What can be checked on this machine before connecting or compiling: the
/// bundle files, the build folder and the unit template.
fn check_local(deployable: &Deployable, report: &Reporter) -> Result<()> {
    deployable.check_bundle()?;
    for (entry, _) in deployable.bundle_status() {
        let path = deployable.folder.join(&entry);
        let what = if path.is_dir() {
            "folder".to_string()
        } else {
            std::fs::metadata(&path)
                .map(|m| human::bytes(m.len()))
                .unwrap_or_default()
        };
        report.log(LogKind::Success, format!("bundle {entry} ({what})"));
    }
    let build_dir = deployable.build_dir();
    if !build_dir.is_dir() {
        bail!("build.cwd {} does not exist", build_dir.display());
    }
    let sample = crate::release::ReleaseName::new("0.0.0", "check", false, chrono::Utc::now());
    unit::render_for(deployable, &sample, None)?;
    Ok(())
}

/// Checks what the build produced: every `build.files` entry exists, and the
/// binary can run on the server. Returns a short summary.
fn check_build(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<String> {
    let files = &deployable.config.build.files;
    deployable.check_files()?;
    let mut summary = Vec::new();
    if let Some(binary) = deployable.binary_path() {
        summary.push(check_binary(&binary, facts, report)?);
    }
    if !files.is_empty() {
        report.log(
            LogKind::Success,
            format!("build.files: {}", files.join(", ")),
        );
        summary.push(count(files.len(), "build output"));
    }
    Ok(summary.join(" · "))
}

/// Checks the built binary can run on the server; returns a short summary.
fn check_binary(binary: &Path, facts: &Facts, report: &Reporter) -> Result<String> {
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

/// Probes once a second until the service has been healthy `STABLE_PROBES`
/// times in a row, it crash-loops, or the timeout passes.
pub(crate) async fn wait_healthy(
    server: &Server,
    health: &HealthConfig,
    report: &Reporter,
    step: usize,
) -> Result<String> {
    let timeout = Duration::from_secs(health.timeout_secs);
    let started = Instant::now();
    let mut baseline = None;
    let mut streak = 0;
    loop {
        let probe = server.probe(health.url.as_deref()).await?;
        if baseline.is_none() {
            baseline = probe.restarts;
        }
        if let (Some(before), Some(now)) = (baseline, probe.restarts)
            && now > before
        {
            bail!(
                "the service crashed and was restarted {} time(s) by systemd",
                now - before
            );
        }
        if probe.state == "failed" {
            bail!("systemd reports the service as failed");
        }
        if probe.http == HttpCheck::NoClient {
            bail!("health.url is set, but the server has neither curl nor wget");
        }
        streak = if probe.healthy() { streak + 1 } else { 0 };

        let http = match &probe.http {
            HttpCheck::Skipped => String::new(),
            HttpCheck::Ok => " · http ok".into(),
            HttpCheck::Failed(why) => format!(" · http: {why}"),
            HttpCheck::NoClient => String::new(),
        };
        let elapsed = started.elapsed();
        report.progress(
            step,
            elapsed.as_secs().min(health.timeout_secs),
            health.timeout_secs,
            Unit::Seconds,
            format!("{}{http} · stable {streak}/{STABLE_PROBES}", probe.state),
        );

        if streak >= STABLE_PROBES {
            let checked = if health.url.is_some() {
                "active, url answers"
            } else {
                "active"
            };
            return Ok(format!("{checked} · {}", human::duration(elapsed)));
        }
        if elapsed >= timeout {
            let why = match probe.http {
                HttpCheck::Failed(why) => format!("; the health url said: {why}"),
                _ => String::new(),
            };
            bail!(
                "not healthy after {}s: systemd says {}{why}",
                health.timeout_secs,
                if probe.state.is_empty() {
                    "nothing"
                } else {
                    &probe.state
                }
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// After a failed restart or health check: show the journal, put
/// `previous` back, and drop the failed release if given.
pub(crate) async fn recover(
    server: &Server,
    report: &Reporter,
    step: usize,
    err: anyhow::Error,
    previous: Option<&str>,
    discard: Option<&str>,
    total: usize,
) -> Outcome {
    report.fail(step, short(&err));
    report.log(LogKind::Error, format!("{err:#}"));
    match server.journal().await {
        Ok(lines) if !lines.is_empty() => {
            report.log(LogKind::Info, "last journal lines:");
            for line in lines {
                report.log(LogKind::Err, line);
            }
        }
        _ => report.log(LogKind::Warn, "could not read the journal"),
    }
    skip_rest(report, step, total, "rolled back");

    let Some(previous) = previous else {
        report.log(
            LogKind::Warn,
            "nothing was live before, so there is no release to roll back to",
        );
        return Outcome::Failed {
            error: format!("{err:#}"),
        };
    };
    report.log(LogKind::Warn, format!("rolling back to {previous}"));
    let restored = async {
        restore(server, previous, report).await?;
        server.restart(report).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let probe = server.probe(None).await?;
        if probe.state != "active" {
            bail!(
                "{previous} did not come back either: systemd says {}",
                probe.state
            );
        }
        Ok(())
    }
    .await;
    match restored {
        Ok(()) => {
            if let Some(id) = discard {
                let _ = server.discard(id, report).await;
            }
            report.log(LogKind::Success, format!("{previous} is live again"));
            Outcome::RolledBack {
                error: format!("{err:#}"),
                to: previous.to_string(),
            }
        }
        Err(rollback_err) => {
            report.log(LogKind::Error, format!("rollback failed: {rollback_err:#}"));
            Outcome::RollbackFailed {
                error: format!("{err:#}"),
                rollback_error: format!("{rollback_err:#}"),
            }
        }
    }
}

/// Points `current` back at `id` and reinstalls its unit.
async fn restore(server: &Server, id: &str, report: &Reporter) -> Result<()> {
    server.switch(id, report).await?;
    server.install_unit(report).await
}

async fn rotate(
    server: &Server,
    deployable: &Deployable,
    report: &Reporter,
    step: usize,
) -> Result<String> {
    let keep = deployable.config.releases;
    let listing = server.list().await?;
    let plan = plan_rotation(&listing, keep);
    if plan.is_empty() {
        let open = listing
            .entries
            .iter()
            .filter(|e| e.kind == crate::remote::Kind::Open)
            .count();
        return Ok(format!("{open}/{} open, nothing to tidy", keep.open));
    }
    let total = plan.len() as u64;
    let mut done = 0;
    for id in &plan.compress {
        report.progress(step, done, total, Unit::Items, format!("compressing {id}"));
        server.compress(id, report).await?;
        done += 1;
    }
    for id in &plan.drop_open {
        report.progress(step, done, total, Unit::Items, format!("deleting {id}"));
        server.drop_open(id, report).await?;
        done += 1;
    }
    for id in &plan.drop_backup {
        report.progress(
            step,
            done,
            total,
            Unit::Items,
            format!("deleting {id}.tar.gz"),
        );
        server.drop_backup(id, report).await?;
        done += 1;
    }
    report.progress(step, done, total, Unit::Items, "done");
    Ok(format!(
        "{} compressed · {} deleted",
        plan.compress.len(),
        plan.drop_open.len() + plan.drop_backup.len()
    ))
}

/// A failure before anything live changed.
fn stop(report: &Reporter, step: usize, total: usize, err: anyhow::Error) -> Outcome {
    report.fail(step, short(&err));
    report.log(LogKind::Error, format!("{err:#}"));
    skip_rest(report, step, total, "stopped");
    Outcome::Failed {
        error: format!("{err:#}"),
    }
}

fn skip_rest(report: &Reporter, failed: usize, total: usize, why: &str) {
    for step in failed + 1..total {
        report.skip(step, why);
    }
}

/// `1 command`, `3 commands`.
fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// The first line of an error, short enough for a step's detail column.
fn short(err: &anyhow::Error) -> String {
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
struct Throttle {
    last_percent: Option<u64>,
    last_at: Option<Instant>,
}

impl Throttle {
    fn ready(&mut self, done: u64, total: u64) -> bool {
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

#[cfg(test)]
#[path = "deploy_test.rs"]
mod tests;
