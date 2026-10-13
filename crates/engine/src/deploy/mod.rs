//! The deploy: preflight, build, pack, upload, switch, restart, health,
//! rotate. A failure after the switch puts the previous release back.

pub mod report;
mod rollback;

pub use rollback::{ROLLBACK_STEPS, rollback};

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use crate::build;
use crate::bundle::{self, unit};
use crate::config::{Deployable, HealthConfig};
use crate::human;
use crate::release::Manifest;
use crate::remote::{Facts, HttpCheck, Server, plan_rotation};
use crate::setup;
use crate::ssh::{Login, Pool};
use report::{LogKind, Outcome, Reporter, Unit};

pub const DEPLOY_STEPS: [&str; 8] = [
    "Preflight",
    "Build",
    "Pack",
    "Upload",
    "Switch",
    "Restart",
    "Health",
    "Rotate",
];
const PREFLIGHT: usize = 0;
const BUILD: usize = 1;
const PACK: usize = 2;
const UPLOAD: usize = 3;
const SWITCH: usize = 4;
const RESTART: usize = 5;
const HEALTH: usize = 6;
const ROTATE: usize = 7;

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

    // 1. Preflight
    report.start(PREFLIGHT, "Preflight: checking the server");
    let (server, facts) = match preflight(deployable, pool, report).await {
        Ok(found) => found,
        Err(err) => return stop(report, PREFLIGHT, err),
    };

    // 2. Build
    report.start(BUILD, "Build: compiling on this machine");
    let name = match build::release_name(deployable).await {
        Ok(name) => name,
        Err(err) => return stop(report, BUILD, err),
    };
    let id = name.id();
    report.release(&id);
    report.log(LogKind::Info, format!("release {id}"));
    if name.dirty {
        report.log(
            LogKind::Warn,
            "the source has uncommitted changes; the release is marked dirty",
        );
    }
    let started = Instant::now();
    if let Err(err) = build::run(deployable, report, BUILD).await {
        return stop(report, BUILD, err);
    }
    let binary = match check_binary(deployable, &facts, report) {
        Ok(summary) => summary,
        Err(err) => return stop(report, BUILD, err),
    };
    report.finish(
        BUILD,
        format!("{binary} · {}", human::duration(started.elapsed())),
    );

    // 3. Pack
    report.start(PACK, "Pack: bundling the release");
    let deployer = build::deployer(&deployable.build_dir()).await;
    let manifest = Manifest::new(&name, &deployer);
    let packed = async {
        let unit = unit::render_for(deployable, &name)?;
        let (owned, reporter) = (deployable.clone(), report.clone());
        let mut throttle = Throttle::default();
        let bundle = tokio::task::spawn_blocking(move || {
            bundle::pack(&owned, &manifest, &unit, |done, total| {
                if throttle.ready(done, total) {
                    reporter.progress(PACK, done, total, Unit::Bytes, "compressing");
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
        Err(err) => return stop(report, PACK, err),
    };
    report.finish(
        PACK,
        format!(
            "{} files · {} → {}",
            bundle.files,
            human::bytes(bundle.input_bytes),
            human::bytes(bundle.data.len() as u64)
        ),
    );

    // 4. Upload and unpack
    report.start(UPLOAD, "Upload: sending the release");
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
                        UPLOAD,
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
        return stop(report, UPLOAD, err);
    }
    report.finish(
        UPLOAD,
        format!(
            "{} in {}",
            human::bytes(bundle.data.len() as u64),
            human::duration(started.elapsed())
        ),
    );

    // 5. Switch
    report.start(SWITCH, "Switch: making it the current release");
    let previous = facts.current.clone();
    let switched = async {
        server.switch(&id, report).await?;
        server.install_unit(report).await
    }
    .await;
    if let Err(err) = switched {
        report.fail(SWITCH, short(&err));
        report.log(LogKind::Error, format!("{err:#}"));
        // Nothing was restarted, so the old process is still serving.
        return match &previous {
            Some(previous) => match restore(&server, previous, report).await {
                Ok(()) => {
                    let _ = server.discard(&id, report).await;
                    skip_rest(report, SWITCH, &DEPLOY_STEPS, "stopped");
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
                skip_rest(report, SWITCH, &DEPLOY_STEPS, "stopped");
                Outcome::Failed {
                    error: format!("{err:#}"),
                }
            }
        };
    }
    report.finish(SWITCH, format!("current → {id}"));

    // 6. Restart
    report.start(RESTART, &format!("Restart: {}", config.remote.service));
    if let Err(err) = server.restart(report).await {
        return recover(
            &server,
            report,
            RESTART,
            err,
            previous.as_deref(),
            Some(&id),
            &DEPLOY_STEPS,
        )
        .await;
    }
    report.finish(RESTART, "");

    // 7. Health
    report.start(HEALTH, "Health: waiting for the new release to settle");
    match wait_healthy(&server, &config.health, report, HEALTH).await {
        Ok(summary) => report.finish(HEALTH, summary),
        Err(err) => {
            return recover(
                &server,
                report,
                HEALTH,
                err,
                previous.as_deref(),
                Some(&id),
                &DEPLOY_STEPS,
            )
            .await;
        }
    }

    // 8. Rotate
    report.start(ROTATE, "Rotate: tidying old releases");
    match rotate(&server, deployable, report).await {
        Ok(summary) => report.finish(ROTATE, summary),
        Err(err) => {
            report.log(LogKind::Warn, format!("rotation: {err:#}"));
            report.warn(ROTATE, short(&err));
        }
    }

    report.log(LogKind::Success, format!("{id} is live"));
    Outcome::Succeeded { release: id }
}

async fn preflight(
    deployable: &Deployable,
    pool: &Pool,
    report: &Reporter,
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
    let facts = server.facts(&endpoint.user).await?;

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
    if let Some(free) = facts.free_bytes
        && free < MIN_FREE
    {
        bail!("only {} free on the server", human::bytes(free));
    }

    report.finish(PREFLIGHT, format!("{} · {glibc} · {free} free", facts.arch));
    Ok((server, facts))
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
    unit::render_for(deployable, &sample)?;
    Ok(())
}

/// Checks the built binary can run on the server; returns a short summary.
fn check_binary(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<String> {
    let info = build::inspect(&deployable.binary_path())?;
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
    steps: &[&str],
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
    skip_rest(report, step, steps, "rolled back");

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

async fn rotate(server: &Server, deployable: &Deployable, report: &Reporter) -> Result<String> {
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
        report.progress(
            ROTATE,
            done,
            total,
            Unit::Items,
            format!("compressing {id}"),
        );
        server.compress(id, report).await?;
        done += 1;
    }
    for id in &plan.drop_open {
        report.progress(ROTATE, done, total, Unit::Items, format!("deleting {id}"));
        server.drop_open(id, report).await?;
        done += 1;
    }
    for id in &plan.drop_backup {
        report.progress(
            ROTATE,
            done,
            total,
            Unit::Items,
            format!("deleting {id}.tar.gz"),
        );
        server.drop_backup(id, report).await?;
        done += 1;
    }
    report.progress(ROTATE, done, total, Unit::Items, "done");
    Ok(format!(
        "{} compressed · {} deleted",
        plan.compress.len(),
        plan.drop_open.len() + plan.drop_backup.len()
    ))
}

/// A failure before anything live changed.
fn stop(report: &Reporter, step: usize, err: anyhow::Error) -> Outcome {
    report.fail(step, short(&err));
    report.log(LogKind::Error, format!("{err:#}"));
    skip_rest(report, step, &DEPLOY_STEPS, "stopped");
    Outcome::Failed {
        error: format!("{err:#}"),
    }
}

fn skip_rest(report: &Reporter, failed: usize, steps: &[&str], why: &str) {
    for step in failed + 1..steps.len() {
        report.skip(step, why);
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
