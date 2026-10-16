//! A deploy runs the pipeline: preflight, build, pack, upload, unpack,
//! setup, backup, switch, restart, health and done, then failed if a step
//! failed. A failure after the switch asks whether to put the previous
//! release back.

pub mod report;
mod rollback;

pub use rollback::{rollback, rollback_step_names, rollback_steps};

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use crate::build;
use crate::bundle;
use crate::bundle::files::{self, Kind, Source};
use crate::config::{
    Commands, DeployConfig, Deployable, Manager, Program, ROLLBACK_ANSWER_SECS, Rollback,
    release_path,
};
use crate::human;
use crate::release::{MANIFEST_FILE, Manifest, ReleaseName};
use crate::remote::{
    CommandEnv, Facts, HttpCheck, Place, Server, Unpacked, check_version, pick, plan_rotation,
};
use crate::ssh::{Pool, ShellEnv};
use report::{LogKind, Outcome, Reporter, Unit};

/// Every step a deploy or rollback can have, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Connect,
    Preflight,
    Build,
    Pack,
    Upload,
    Unpack,
    Setup,
    Backup,
    Switch,
    Restart,
    Health,
    Done,
    Failed,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Self::Connect => "Connect",
            Self::Preflight => "Preflight",
            Self::Build => "Build",
            Self::Pack => "Pack",
            Self::Upload => "Upload",
            Self::Unpack => "Unpack",
            Self::Setup => "Setup",
            Self::Backup => "Backup",
            Self::Switch => "Switch",
            Self::Restart => "Restart",
            Self::Health => "Health",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }

    /// As deploy.yaml and `SHIPDECK_STEP` spell it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Preflight => "preflight",
            Self::Build => "build",
            Self::Pack => "pack",
            Self::Upload => "upload",
            Self::Unpack => "unpack",
            Self::Setup => "setup",
            Self::Backup => "backup",
            Self::Switch => "switch",
            Self::Restart => "restart",
            Self::Health => "health",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// The steps a deploy of `config` goes through. Failed is listed only when
/// `pipeline.failed` has commands.
pub fn deploy_steps(config: &DeployConfig) -> Vec<Step> {
    let mut steps = vec![
        Step::Preflight,
        Step::Build,
        Step::Pack,
        Step::Upload,
        Step::Unpack,
        Step::Setup,
        Step::Backup,
        Step::Switch,
        Step::Restart,
        Step::Health,
        Step::Done,
    ];
    if !config.pipeline.failed.run.local.is_empty() {
        steps.push(Step::Failed);
    }
    steps
}

/// The names of [`deploy_steps`], for the screen.
pub fn deploy_step_names(config: &DeployConfig) -> Vec<&'static str> {
    deploy_steps(config).into_iter().map(Step::name).collect()
}

/// What `SHIPDECK_ACTION` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Deploy,
    Rollback,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::Rollback => "rollback",
        }
    }
}

/// Below this much free space on the server, preflight stops.
const MIN_FREE: u64 = 100 * 1024 * 1024;
/// What shipdeck itself runs on the server, with the package each comes
/// in: the bundle is unpacked with `tar -I zstd` and checked with
/// `sha256sum`.
const SERVER_TOOLS: [(&str, &str); 3] =
    [("tar", "tar"), ("zstd", "zstd"), ("sha256sum", "coreutils")];
/// Consecutive healthy probes, one second apart, before a release counts as up.
const STABLE_PROBES: u32 = 3;
/// The pause before a failed health check runs again.
const CHECK_RETRY: Duration = Duration::from_secs(2);

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

    /// What `current` already has of `contents`, hashed by the server just
    /// now. Nothing, with a note why, when nothing is live or the server
    /// cannot say: then every file is sent.
    async fn on_server(
        &self,
        server: &Server,
        id: &str,
        contents: &bundle::Contents,
        live: Option<&str>,
    ) -> HashMap<String, String> {
        let report = self.report;
        let Some(live) = live else {
            report.log(LogKind::Info, "nothing is live yet, so every file is sent");
            return HashMap::new();
        };
        let candidates = contents.candidates();
        report.progress(
            self.at(Step::Pack),
            0,
            0,
            Unit::Items,
            format!("asking {live} which files it has"),
        );
        match server.current_hashes(id, &candidates).await {
            Ok(found) => found,
            Err(err) => {
                report.log(
                    LogKind::Info,
                    format!("{err:#}; every file is sent instead"),
                );
                HashMap::new()
            }
        }
    }

    /// Uploads `bundle` into `releases/`, with the receipt and the lists of
    /// files to copy from `current`; progress goes to `row`.
    async fn send(
        &self,
        server: &Server,
        id: &str,
        bundle: &bundle::Bundle,
        row: Step,
    ) -> Result<()> {
        let report = self.report;
        let index = self.at(row);
        let started = Instant::now();
        server.prepare(report).await?;
        let target = server.upload_path(id);
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
                        index,
                        sent,
                        total,
                        Unit::Bytes,
                        format!("{}/s", human::bytes(speed as u64)),
                    );
                }
            })
            .await?;
        server.upload_plan(id, &bundle.receipt, &bundle.reuse).await
    }

    /// Unpacks `bundle` into `releases/<id>` and checks every file. When a
    /// file copied from `current` does not match, the whole release is
    /// packed, sent and unpacked once more. Returns the step's summary.
    async fn unpack(
        &self,
        server: &Server,
        id: &str,
        contents: &Arc<bundle::Contents>,
        manifest: &Manifest,
        bundle: &bundle::Bundle,
    ) -> Result<String> {
        let report = self.report;
        let modes: Vec<u32> = bundle.reuse.keys().copied().collect();
        let names = match server.unpack(id, &modes, report).await? {
            Unpacked::Verified if bundle.reused == 0 => {
                return Ok(format!("{} unpacked · verified", bundle.sent()));
            }
            Unpacked::Verified => {
                return Ok(format!(
                    "{} unpacked · {} copied · verified",
                    bundle.sent(),
                    bundle.reused
                ));
            }
            Unpacked::Mismatch(names) => names,
        };
        if bundle.reused == 0 {
            bail!(
                "releases/{id} does not match what was packed: {}",
                some(&names)
            );
        }
        report.log(
            LogKind::Warn,
            if names.is_empty() {
                "copying files from current failed; sending the whole release".to_string()
            } else {
                format!(
                    "{} copied from current did not match: {}; sending the whole release",
                    count(names.len(), "file"),
                    some(&names)
                )
            },
        );
        let whole = self
            .pack(contents, manifest, HashMap::new(), Step::Unpack)
            .await?;
        self.send(server, id, &whole, Step::Unpack).await?;
        match server.unpack(id, &[], report).await? {
            Unpacked::Verified => Ok(format!("{} unpacked · verified · sent whole", whole.sent())),
            Unpacked::Mismatch(names) => bail!(
                "releases/{id} does not match what was packed, even sent whole: {}",
                some(&names)
            ),
        }
    }

    /// Packs `contents` in the background, leaving out what `on_server`
    /// already has; progress goes to `row`.
    async fn pack(
        &self,
        contents: &Arc<bundle::Contents>,
        manifest: &Manifest,
        on_server: HashMap<String, String>,
        row: Step,
    ) -> Result<bundle::Bundle> {
        let index = self.at(row);
        let (contents, manifest, reporter) =
            (contents.clone(), manifest.clone(), self.report.clone());
        let mut throttle = Throttle::default();
        tokio::task::spawn_blocking(move || {
            bundle::pack(&contents, &manifest, &on_server, |done, total| {
                if throttle.ready(done, total) {
                    reporter.progress(index, done, total, Unit::Bytes, "compressing");
                }
            })
        })
        .await
        .map_err(|err| anyhow!("packing stopped: {err}"))?
    }

    /// Whether `pipeline.setup` has anything to run.
    fn has_setup(&self) -> bool {
        let setup = &self.config().pipeline.setup;
        !(setup.run.server.is_empty() && setup.check.server.is_empty())
    }

    /// Runs `pipeline.setup` in `releases/<id>`: `run.server`, then
    /// `check.server`. Returns the step's summary.
    async fn set_up(&self, server: &Server, id: &str) -> Result<String> {
        let setup = &self.config().pipeline.setup;
        let started = Instant::now();
        let place = Place::Release(id.to_string());
        self.remote(server, place.clone(), Step::Setup, &setup.run.server)
            .await?;
        self.remote(server, place, Step::Setup, &setup.check.server)
            .await
            .map_err(|err| err.context("a setup check failed"))?;
        Ok(format!(
            "{} · {}",
            count(setup.run.server.len() + setup.check.server.len(), "command"),
            human::duration(started.elapsed())
        ))
    }

    /// Packs what `pipeline.backup.files` picks from `releases/<id>` into
    /// `backups/<id>.tar.zst`. `deployed` is every path the bundle had.
    async fn back_up(&self, server: &Server, id: &str, deployed: &[String]) -> Result<String> {
        let config = self.config();
        let report = self.report;
        let listing = server.release_files(id).await?;
        let own: Vec<String> = std::iter::once(MANIFEST_FILE.to_string())
            .chain(config.service.own_file())
            .collect();
        let shared: Vec<String> = config
            .service
            .shared
            .iter()
            .map(|s| release_path(s))
            .collect();
        let deployed: BTreeSet<String> = deployed.iter().cloned().collect();
        let picked = pick(
            &listing,
            &deployed,
            &own,
            &shared,
            &config.pipeline.backup.files,
        )?;
        for pattern in &picked.unmatched {
            report.log(
                LogKind::Warn,
                format!(
                    "pipeline.backup.files.include: {pattern} matched nothing in releases/{id}"
                ),
            );
        }
        let folders: HashSet<&str> = listing
            .iter()
            .filter(|f| f.is_dir)
            .map(|f| f.path.as_str())
            .collect();
        let kept = picked
            .paths
            .iter()
            .filter(|p| !folders.contains(p.as_str()))
            .count();
        let all = listing.iter().filter(|f| !f.is_dir).count();
        let bytes = server.back_up(id, &picked.paths, report).await?;
        report.log(
            LogKind::Info,
            format!(
                "backups/{id}.tar.zst holds {kept} of the release's {} ({})",
                count(all, "file"),
                human::bytes(bytes)
            ),
        );
        Ok(format!("{} · {}", count(kept, "file"), human::bytes(bytes)))
    }

    /// Removes a deploy's new release before the switch; the live one is
    /// untouched.
    async fn remove_new(&self, server: &Server, id: &str) {
        let _ = server.discard(id, self.report).await;
        let live = match &self.previous {
            Some(previous) => format!("; {previous} is still live"),
            None => String::new(),
        };
        self.report
            .log(LogKind::Info, format!("releases/{id} was removed{live}"));
    }

    /// Waits until the service is up, then runs the health checks, trying
    /// them again until `timeout_secs` has passed.
    async fn health(&self, server: &Server) -> Result<String> {
        let health = &self.config().pipeline.health;
        let index = self.at(Step::Health);
        let started = Instant::now();
        let summary = wait_healthy(server, health, self.report, index).await?;
        let checks = &health.check;
        if checks.server.is_empty() && checks.local.is_empty() {
            return Ok(summary);
        }
        let deadline = started + Duration::from_secs(health.timeout_secs);
        loop {
            let mut passed = self
                .remote(server, Place::Current, Step::Health, &checks.server)
                .await;
            if passed.is_ok() {
                passed = self
                    .local(Step::Health, Step::Health, None, &checks.local)
                    .await;
            }
            match passed {
                Ok(()) => {
                    return Ok(format!(
                        "{summary} · {} passed",
                        count(checks.server.len() + checks.local.len(), "check")
                    ));
                }
                Err(err) if Instant::now() + CHECK_RETRY < deadline => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    self.report.log(
                        LogKind::Warn,
                        format!(
                            "{err:#}; trying again in {}s ({}s left)",
                            CHECK_RETRY.as_secs(),
                            left.as_secs()
                        ),
                    );
                    tokio::time::sleep(CHECK_RETRY).await;
                }
                Err(err) => {
                    return Err(err.context(format!(
                        "the health checks still fail after {}s",
                        health.timeout_secs
                    )));
                }
            }
        }
    }

    /// The release is live: tidy old releases after a deploy, then run
    /// `pipeline.done`. Nothing here changes what is live.
    async fn done(&self, server: &Server) {
        let report = self.report;
        let id = self.release.clone().unwrap_or_default();
        report.log(LogKind::Success, format!("{id} is live"));
        let run = &self.config().pipeline.done.run;
        if self.action == Action::Rollback && run.server.is_empty() && run.local.is_empty() {
            self.skip(Step::Done, "nothing to run");
            return;
        }
        self.start(
            Step::Done,
            if self.action == Action::Deploy {
                "tidying old releases"
            } else {
                "running pipeline.done"
            },
        );
        let index = self.at(Step::Done);
        let mut notes = Vec::new();
        let mut problem = None;
        if self.action == Action::Deploy {
            match rotate(server, self.deployable, report, index).await {
                Ok(summary) => notes.push(summary),
                Err(err) => {
                    report.log(LogKind::Warn, format!("tidying: {err:#}"));
                    problem = Some(err);
                }
            }
        }
        let mut ran = self
            .remote(server, Place::Current, Step::Done, &run.server)
            .await;
        if ran.is_ok() {
            ran = self.local(Step::Done, Step::Done, None, &run.local).await;
        }
        match ran {
            Ok(()) if !(run.server.is_empty() && run.local.is_empty()) => {
                notes.push(count(run.server.len() + run.local.len(), "command"));
            }
            Ok(()) => {}
            Err(err) => {
                report.log(
                    LogKind::Warn,
                    format!("pipeline.done: {err:#}; {id} stays live"),
                );
                problem = Some(err);
            }
        }
        match problem {
            Some(err) => report.warn(index, short(&err)),
            None => self.finish(Step::Done, notes.join(" · ")),
        }
    }

    /// Runs `pipeline.failed` when a step failed. Its own failure is only
    /// logged.
    pub(crate) async fn failed(&self, ended: &Ended) {
        if !self.steps.contains(&Step::Failed) {
            return;
        }
        let Some((step, error)) = &ended.failed else {
            self.skip(Step::Failed, "nothing failed");
            return;
        };
        self.start(
            Step::Failed,
            &format!("running pipeline.failed (after {})", step.key()),
        );
        let first = error.lines().next().unwrap_or_default();
        let commands = &self.config().pipeline.failed.run.local;
        match self.local(Step::Failed, *step, Some(first), commands).await {
            Ok(()) => self.finish(Step::Failed, count(commands.len(), "command")),
            Err(err) => {
                self.report
                    .log(LogKind::Warn, format!("pipeline.failed: {err:#}"));
                self.report.warn(self.at(Step::Failed), short(&err));
            }
        }
    }

    /// After a failed switch, restart or health check: show the journal,
    /// ask (or not) and put the previous release back.
    async fn recover(&self, server: &Server, step: Step, err: anyhow::Error) -> Ended {
        let report = self.report;
        report.fail(self.at(step), short(&err));
        report.log(LogKind::Error, format!("{err:#}"));
        match server.journal().await {
            Ok(lines) if !lines.is_empty() => {
                report.log(LogKind::Info, "last log lines:");
                for line in lines {
                    report.log(LogKind::Err, line);
                }
            }
            _ => report.log(LogKind::Warn, "could not read the service's log"),
        }
        let error = format!("{err:#}");
        let id = self.release.clone().unwrap_or_default();
        let manager = server.manager().label();

        let Some(previous) = self.previous.clone().filter(|p| *p != id) else {
            report.log(
                LogKind::Warn,
                "nothing was live before, so there is nothing to roll back to",
            );
            self.skip_rest(step, "stopped");
            return Ended {
                outcome: Outcome::Failed {
                    error: error.clone(),
                },
                failed: Some((step, error)),
            };
        };
        if !self.should_roll_back(step, &err, &previous).await {
            report.log(
                LogKind::Warn,
                format!(
                    "{id} stays live as you chose; roll back any time from the Releases screen"
                ),
            );
            self.skip_rest(step, "kept as it is");
            return Ended {
                outcome: Outcome::Kept {
                    release: id,
                    error: error.clone(),
                },
                failed: Some((step, error)),
            };
        }
        self.skip_rest(step, "rolled back");
        report.log(LogKind::Warn, format!("rolling back to {previous}"));
        let restored = async {
            restore(server, &previous, report).await?;
            server.restart(report).await?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let probe = server.probe(None).await?;
            if probe.state != "active" {
                bail!(
                    "{previous} did not come back either: {manager} says {}",
                    probe.state
                );
            }
            Ok(())
        }
        .await;
        let outcome = match restored {
            Ok(()) => {
                // A deploy's failed release goes; a rollback's target was an
                // old release and stays.
                if self.action == Action::Deploy {
                    let _ = server.discard(&id, report).await;
                }
                report.log(LogKind::Success, format!("{previous} is live again"));
                Outcome::RolledBack {
                    error: error.clone(),
                    to: previous,
                }
            }
            Err(rollback_err) => {
                report.log(LogKind::Error, format!("rollback failed: {rollback_err:#}"));
                Outcome::RollbackFailed {
                    error: error.clone(),
                    rollback_error: format!("{rollback_err:#}"),
                }
            }
        };
        Ended {
            outcome,
            failed: Some((step, error)),
        }
    }

    /// `pipeline.failed.rollback`: auto rolls back; ask waits for an answer
    /// and rolls back when none comes in time.
    async fn should_roll_back(&self, step: Step, err: &anyhow::Error, previous: &str) -> bool {
        let report = self.report;
        if self.config().pipeline.failed.rollback == Rollback::Auto {
            return true;
        }
        let id = self.release.clone().unwrap_or_default();
        report.log(
            LogKind::Info,
            format!(
                "asking whether to roll back to {previous}; with no answer in {ROLLBACK_ANSWER_SECS}s it rolls back"
            ),
        );
        let answer = report.ask_rollback(
            &id,
            previous,
            step.name(),
            &short(err),
            ROLLBACK_ANSWER_SECS,
        );
        match tokio::time::timeout(Duration::from_secs(ROLLBACK_ANSWER_SECS), answer).await {
            Ok(Ok(true)) => {
                report.log(LogKind::Info, "you chose to roll back");
                true
            }
            Ok(Ok(false)) => {
                report.log(LogKind::Warn, format!("you chose to keep {id}"));
                false
            }
            // Nobody is there to answer.
            Ok(Err(_)) => true,
            Err(_) => {
                report.log(
                    LogKind::Warn,
                    format!("no answer in {ROLLBACK_ANSWER_SECS}s; rolling back"),
                );
                true
            }
        }
    }
}

async fn preflight(
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
fn check_tools(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<()> {
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

/// PM2 is a version shipdeck knows, and no systemd unit of the same name
/// still runs the app.
fn check_pm2(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<()> {
    let output = facts.pm2_version.as_deref().unwrap_or_default();
    let version = check_version(output).map_err(|why| anyhow!(why))?;
    report.log(LogKind::Success, format!("pm2 {version}"));
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
fn start_path(deployable: &Deployable, facts: &Facts) -> Option<String> {
    match deployable.start() {
        (Program::Tool(tool), _) => facts.tool_path(&tool).map(str::to_string),
        _ => None,
    }
}

/// What can be checked on this machine before connecting: the project
/// folder and the service file.
fn check_local(deployable: &Deployable) -> Result<()> {
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
fn check_build(deployable: &Deployable, facts: &Facts, report: &Reporter) -> Result<String> {
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
    health: &crate::config::Health,
    report: &Reporter,
    step: usize,
) -> Result<String> {
    let timeout = Duration::from_secs(health.timeout_secs);
    let started = Instant::now();
    let manager = server.manager().label();
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
                "the service crashed and was restarted {} time(s) by {manager}",
                now - before
            );
        }
        if probe.state == "failed" {
            bail!("{manager} reports the service as failed");
        }
        if probe.http == HttpCheck::NoClient {
            bail!("pipeline.health.url is set, but the server has neither curl nor wget");
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
                "not healthy after {}s: {manager} says {}{why}",
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
    let keep = deployable.config.pipeline.backup.keep;
    let listing = server.list().await?;
    let plan = plan_rotation(&listing, keep);
    if plan.is_empty() {
        let open = listing
            .entries
            .iter()
            .filter(|e| e.kind == crate::remote::Kind::Open)
            .count();
        return Ok(format!("{open}/{} open", keep.open));
    }
    let total = plan.len() as u64;
    let mut done = 0;
    for id in &plan.close {
        report.progress(
            step,
            done,
            total,
            Unit::Items,
            format!("deleting releases/{id}; its backup stays"),
        );
        server.drop_open(id, report).await?;
        done += 1;
    }
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
            format!("deleting the backup of {id}"),
        );
        server.drop_backup(id, report).await?;
        done += 1;
    }
    report.progress(step, done, total, Unit::Items, "tidied");
    let deleted: BTreeSet<&String> = plan.drop_open.iter().chain(&plan.drop_backup).collect();
    Ok(format!(
        "{} backed up only · {} deleted",
        plan.close.len() + plan.compress.len(),
        deleted.len()
    ))
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
