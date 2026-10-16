//! A deploy runs the pipeline: preflight, build, pack, upload, unpack,
//! setup, backup, switch, restart, health and done, then failed if a step
//! failed. A failure after the switch asks whether to put the previous
//! release back.

mod backup;
mod health;
mod preflight;
mod recover;
pub mod report;
mod rollback;
mod ship;
mod steps;

pub use rollback::{rollback, rollback_step_names, rollback_steps};

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use crate::build;
use crate::bundle;
use crate::bundle::files::{self, Kind, Source};
use crate::config::{Commands, DeployConfig, Deployable, release_path};
use crate::human;
use crate::release::Manifest;
use crate::remote::{CommandEnv, Place, Server};
use crate::ssh::Pool;
use backup::restore;
use preflight::{check_build, preflight, start_path};
use report::{LogKind, Outcome, Reporter, Unit};
pub use steps::{Action, Step, deploy_step_names, deploy_steps};

pub async fn deploy(deployable: Deployable, pool: Arc<Pool>, report: Reporter) {
    let mut run = Run::new(
        &deployable,
        &report,
        deploy_steps(&deployable.config),
        Action::Deploy,
    );
    let ended = run.deploy(&pool).await;
    run.failed(&ended).await;
    report.done(ended.outcome);
}

/// How a run ended, and the step that failed if one did.
pub(crate) struct Ended {
    outcome: Outcome,
    failed: Option<(Step, String)>,
}

impl Ended {
    fn ok(release: &str) -> Self {
        Self {
            outcome: Outcome::Succeeded {
                release: release.to_string(),
            },
            failed: None,
        }
    }
}

/// One deploy or rollback in progress: its steps and what is known so far.
pub(crate) struct Run<'a> {
    deployable: &'a Deployable,
    report: &'a Reporter,
    steps: Vec<Step>,
    action: Action,
    /// The release this run puts live, once known.
    release: Option<String>,
    version: Option<String>,
    /// The release that was live before.
    previous: Option<String>,
}

impl<'a> Run<'a> {
    pub(crate) fn new(
        deployable: &'a Deployable,
        report: &'a Reporter,
        steps: Vec<Step>,
        action: Action,
    ) -> Self {
        Self {
            deployable,
            report,
            steps,
            action,
            release: None,
            version: None,
            previous: None,
        }
    }

    fn config(&self) -> &'a DeployConfig {
        &self.deployable.config
    }

    fn at(&self, step: Step) -> usize {
        self.steps
            .iter()
            .position(|s| *s == step)
            .expect("the plan has every step the run reports")
    }

    fn start(&self, step: Step, what: &str) {
        self.report
            .start(self.at(step), &format!("{}: {what}", step.name()));
    }

    fn finish(&self, step: Step, detail: impl Into<String>) {
        self.report.finish(self.at(step), detail);
    }

    fn skip(&self, step: Step, detail: impl Into<String>) {
        self.report.skip(self.at(step), detail);
    }

    /// Marks every step after `failed` skipped, except Failed itself.
    fn skip_rest(&self, failed: Step, why: &str) {
        for (index, step) in self.steps.iter().enumerate().skip(self.at(failed) + 1) {
            if *step != Step::Failed {
                self.report.skip(index, why);
            }
        }
    }

    /// A failure before anything live changed.
    fn stop(&self, step: Step, err: anyhow::Error) -> Ended {
        self.report.fail(self.at(step), short(&err));
        self.report.log(LogKind::Error, format!("{err:#}"));
        self.skip_rest(step, "stopped");
        let error = format!("{err:#}");
        Ended {
            outcome: Outcome::Failed {
                error: error.clone(),
            },
            failed: Some((step, error)),
        }
    }

    /// `SHIPDECK_*` for a command in `step`.
    fn vars(&self, step: Step, error: Option<&str>) -> Vec<(&'static str, String)> {
        let config = self.config();
        let mut vars = vec![
            ("SHIPDECK_ACTION", self.action.key().to_string()),
            ("SHIPDECK_STEP", step.key().to_string()),
            ("SHIPDECK_NAME", config.name.clone()),
            ("SHIPDECK_SERVICE", config.service.name.clone()),
            ("SHIPDECK_HOST", self.deployable.endpoint.name.clone()),
            ("SHIPDECK_APP_DIR", config.service.dir.clone()),
            (
                "SHIPDECK_SHARED_DIR",
                format!("{}/shared", config.service.dir),
            ),
        ];
        if let Some(id) = &self.release {
            vars.push(("SHIPDECK_RELEASE", id.clone()));
        }
        if let Some(version) = &self.version {
            vars.push(("SHIPDECK_VERSION", version.clone()));
        }
        if let Some(previous) = &self.previous {
            vars.push(("SHIPDECK_PREVIOUS", previous.clone()));
        }
        if let Some(error) = error {
            vars.push(("SHIPDECK_ERROR", error.to_string()));
        }
        vars
    }

    /// Runs `commands` on this machine in `project`, one after another.
    /// Progress goes to `row`; `SHIPDECK_STEP` says `step`.
    async fn local(
        &self,
        row: Step,
        step: Step,
        error: Option<&str>,
        commands: &Commands,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }
        let dir = self.deployable.project_dir();
        if !dir.is_dir() {
            bail!("the project folder {} does not exist", dir.display());
        }
        let absolute = |path: &Path| {
            std::path::absolute(path)
                .unwrap_or(path.to_path_buf())
                .display()
                .to_string()
        };
        let mut env = self.vars(step, error);
        env.push(("SHIPDECK_FOLDER", absolute(&self.deployable.folder)));
        env.push(("SHIPDECK_PROJECT", absolute(&dir)));
        let index = self.at(row);
        for (done, command) in commands.iter().enumerate() {
            self.report.progress(
                index,
                done as u64,
                commands.len() as u64,
                Unit::Items,
                command.trim(),
            );
            build::run_local(command, &dir, &env, self.report, index).await?;
        }
        Ok(())
    }

    /// Runs `commands` on the server at `place`, one after another.
    async fn remote(
        &self,
        server: &Server,
        place: Place,
        step: Step,
        commands: &Commands,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }
        let service = &self.config().service;
        let env = CommandEnv {
            service: service.env.clone(),
            env_file: service.env_file.as_deref().map(release_path),
            vars: self.vars(step, None),
        };
        let index = self.at(step);
        for (done, command) in commands.iter().enumerate() {
            self.report.progress(
                index,
                done as u64,
                commands.len() as u64,
                Unit::Items,
                command.trim(),
            );
            server.command(&place, command, &env, self.report).await?;
        }
        Ok(())
    }

    async fn deploy(&mut self, pool: &Pool) -> Ended {
        let deployable = self.deployable;
        let pipeline = &self.config().pipeline;
        let report = self.report;

        // Preflight: shipdeck's checks, then the server's commands, then
        // this machine's. Nothing is built or changed yet.
        self.start(Step::Preflight, "checking your PC and the server");
        let index = self.at(Step::Preflight);
        let (server, facts, summary) = match preflight(deployable, pool, report, index).await {
            Ok(found) => found,
            Err(err) => return self.stop(Step::Preflight, err),
        };
        self.previous = facts.current.clone();
        let commands = &pipeline.preflight.run;
        if let Err(err) = self
            .remote(&server, Place::AppDir, Step::Preflight, &commands.server)
            .await
        {
            return self.stop(Step::Preflight, err);
        }
        if let Err(err) = self
            .local(Step::Preflight, Step::Preflight, None, &commands.local)
            .await
        {
            return self.stop(Step::Preflight, err);
        }
        // Files from the deploy folder are there before the build; project
        // files are checked once it has run.
        match files::walk_source(deployable, Source::Deploy) {
            Ok(items) => {
                for (path, item) in &items {
                    if let Kind::File { size, .. } = item.kind {
                        report.log(
                            LogKind::Success,
                            format!("deploy folder: {path} ({})", human::bytes(size)),
                        );
                    }
                }
            }
            Err(err) => return self.stop(Step::Preflight, err),
        }
        self.finish(Step::Preflight, summary);

        // Build
        self.start(Step::Build, "building on your PC");
        let (name, source) = match build::release_name(deployable).await {
            Ok(found) => found,
            Err(err) => return self.stop(Step::Build, err),
        };
        let id = name.id();
        self.release = Some(id.clone());
        self.version = Some(name.version.clone());
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
        let build_commands = &pipeline.build.run.local;
        if build_commands.is_empty() {
            report.log(
                LogKind::Info,
                "pipeline.build.run.local is empty; shipping the files as they are",
            );
        } else if let Err(err) = self
            .local(Step::Build, Step::Build, None, build_commands)
            .await
        {
            return self.stop(Step::Build, err.context("the build failed"));
        }
        let built = match check_build(deployable, &facts, report) {
            Ok(summary) => summary,
            Err(err) => return self.stop(Step::Build, err),
        };
        self.finish(
            Step::Build,
            format!("{built} · {}", human::duration(started.elapsed())),
        );

        // Pack
        self.start(Step::Pack, "packing the release");
        let index = self.at(Step::Pack);
        let deployer = build::deployer(&deployable.project_dir()).await;
        let mut manifest = Manifest::new(&name, &deployer);
        let gathered = async {
            let start_path = start_path(deployable, &facts);
            let own = bundle::service_file(deployable, &name, start_path.as_deref())?;
            report.log(
                LogKind::Info,
                format!(
                    "start: {}",
                    bundle::start_line(deployable, start_path.as_deref())
                ),
            );
            let (owned, reporter) = (deployable.clone(), report.clone());
            let mut throttle = Throttle::default();
            let contents = tokio::task::spawn_blocking(move || {
                bundle::gather(&owned, own.as_ref(), |done, total| {
                    if throttle.ready(done, total) {
                        reporter.progress(index, done, total, Unit::Bytes, "hashing");
                    }
                })
            })
            .await
            .map_err(|err| anyhow!("packing stopped: {err}"))??;
            let fingerprint = contents.fingerprint(deployable, start_path.as_deref())?;
            Ok::<_, anyhow::Error>((Arc::new(contents), fingerprint))
        }
        .await;
        let (contents, fingerprint) = match gathered {
            Ok(found) => found,
            Err(err) => return self.stop(Step::Pack, err),
        };
        manifest.content = Some(fingerprint);
        // A live release from before fingerprints has none: always a change.
        if let Some(live) = facts.current.as_deref()
            && facts.content == manifest.content
            && !self.deploy_unchanged(live).await
        {
            return self.unchanged(Step::Pack, live);
        }
        let packed = async {
            let on_server = self
                .on_server(&server, &id, &contents, facts.current.as_deref())
                .await;
            let bundle = self
                .pack(&contents, &manifest, on_server, Step::Pack)
                .await?;
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
            Err(err) => return self.stop(Step::Pack, err),
        };
        for path in &contents.unused_ecosystem {
            report.log(
                LogKind::Warn,
                format!(
                    "{path} is in the release but PM2 runs {} from pm2.options; \
                     to use your file, set pm2.ecosystem: {path} and remove pm2.options and service.start",
                    self.config().service.pm2_file()
                ),
            );
        }
        for path in &contents.replaced {
            report.log(
                LogKind::Info,
                format!("{path} from the deploy folder replaces the project's {path}"),
            );
        }
        let sizes = format!(
            "{} → {}",
            human::bytes(bundle.input_bytes),
            human::bytes(bundle.data.len() as u64)
        );
        self.finish(
            Step::Pack,
            if bundle.reused == 0 {
                format!("{} · {sizes}", count(bundle.files, "file"))
            } else {
                format!(
                    "{} · {} already on the server · {} to send · {sizes}",
                    count(bundle.files, "file"),
                    bundle.reused,
                    bundle.sent()
                )
            },
        );

        // Upload
        self.start(Step::Upload, "sending the release");
        let started = Instant::now();
        if let Err(err) = self.send(&server, &id, &bundle, Step::Upload).await {
            let _ = server.discard(&id, report).await;
            return self.stop(Step::Upload, err);
        }
        self.finish(
            Step::Upload,
            format!(
                "{} in {}",
                human::bytes(bundle.data.len() as u64),
                human::duration(started.elapsed())
            ),
        );

        // Unpack
        self.start(Step::Unpack, &format!("unpacking into releases/{id}"));
        let shared = &self.config().service.shared;
        let unpacked = async {
            let summary = self
                .unpack(&server, &id, &contents, &manifest, &bundle)
                .await?;
            server.link_shared(&id, shared, report).await?;
            Ok::<_, anyhow::Error>(summary)
        }
        .await;
        let summary = match unpacked {
            Ok(summary) => summary,
            Err(err) => {
                let _ = server.discard(&id, report).await;
                return self.stop(Step::Unpack, err);
            }
        };
        self.finish(
            Step::Unpack,
            if shared.is_empty() {
                format!("releases/{id} · {summary}")
            } else {
                format!(
                    "releases/{id} · {summary} · {} linked",
                    count(shared.len(), "shared folder")
                )
            },
        );

        // Setup: in the new release, while the old one still serves.
        if self.has_setup() {
            self.start(Step::Setup, &format!("setting up releases/{id}"));
            match self.set_up(&server, &id).await {
                Ok(summary) => self.finish(Step::Setup, summary),
                Err(err) => {
                    self.remove_new(&server, &id).await;
                    return self.stop(Step::Setup, err);
                }
            }
        } else {
            self.skip(Step::Setup, "nothing to run");
        }

        // Backup: what stays of the release once its folder goes.
        if pipeline.backup.keep.compressed == 0 {
            self.skip(Step::Backup, "keep.compressed is 0");
        } else {
            self.start(Step::Backup, &format!("packing backups/{id}.tar.zst"));
            match self.back_up(&server, &id, &bundle.paths).await {
                Ok(summary) => self.finish(Step::Backup, summary),
                Err(err) => {
                    self.remove_new(&server, &id).await;
                    return self.stop(Step::Backup, err);
                }
            }
        }

        // Switch
        self.start(Step::Switch, "making it the current release");
        let switched = async {
            server.switch(&id, report).await?;
            server.install_unit(report).await
        }
        .await;
        if let Err(err) = switched {
            return self.recover(&server, Step::Switch, err).await;
        }
        self.finish(Step::Switch, format!("current → {id}"));

        // Restart
        self.start(Step::Restart, &self.config().service.name);
        if let Err(err) = server.restart(report).await {
            return self.recover(&server, Step::Restart, err).await;
        }
        self.finish(Step::Restart, "");

        // Health
        self.start(Step::Health, "waiting for the new release to settle");
        match self.health(&server).await {
            Ok(summary) => self.finish(Step::Health, summary),
            Err(err) => return self.recover(&server, Step::Health, err).await,
        }

        self.done(&server).await;
        Ended::ok(&id)
    }

    /// Asks whether to deploy a release `live` already holds. With nobody
    /// to answer, it does not.
    async fn deploy_unchanged(&self, live: &str) -> bool {
        let report = self.report;
        let id = self.release.clone().unwrap_or_default();
        report.log(
            LogKind::Info,
            format!(
                "nothing changed since {live}, the live release; asking whether to deploy anyway"
            ),
        );
        match report.ask_unchanged(&id, live).await {
            Ok(true) => {
                report.log(
                    LogKind::Info,
                    format!("deploying anyway, as a duplicate of {live}"),
                );
                true
            }
            Ok(false) | Err(_) => false,
        }
    }

    /// Stops a deploy whose release `live` already holds; nothing on the
    /// server was touched.
    fn unchanged(&self, step: Step, live: &str) -> Ended {
        self.report.skip(self.at(step), "nothing changed");
        self.skip_rest(step, "nothing changed");
        self.report.log(
            LogKind::Info,
            format!("nothing was deployed; {live} stays live"),
        );
        Ended {
            outcome: Outcome::Unchanged {
                live: live.to_string(),
            },
            failed: None,
        }
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

/// Up to five names, and how many more.
fn some(names: &[String]) -> String {
    match names.len() {
        0 => "no file named".to_string(),
        n if n <= 5 => names.join(", "),
        n => format!("{} and {} more", names[..5].join(", "), n - 5),
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
