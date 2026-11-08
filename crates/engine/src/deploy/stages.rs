//! The deploy steps after the project lock: preflight, build, pack, upload,
//! unpack, setup, backup, switch, restart and health.

use anyhow::anyhow;

use super::{
    Arc, Bundle, Contents, Ended, Facts, Instant, Kind, LogKind, Manifest, Place, Pool,
    ReleaseName, Result, Run, Server, Source, Step, Throttle, Unit, bail, build, bundle,
    check_build, count, files, human, preflight, restart_processes, start_path, switch_processes,
};

impl Run<'_> {
    /// Preflight: shipdeck's checks, then the server's commands, then this
    /// machine's. Nothing is built or changed yet.
    pub(super) async fn step_preflight(&mut self, pool: &Pool) -> Result<(Server, Facts), Ended> {
        self.start(Step::Preflight, "checking your PC and the server");
        let index = self.at(Step::Preflight);
        let (server, mut facts, summary) =
            match preflight(self.deployable, pool, self.report, index).await {
                Ok(found) => found,
                Err(err) => return Err(self.stop(Step::Preflight, &err)),
            };
        self.project_lock = Some(
            crate::agent::ProjectGuard::acquire(&server)
                .await
                .map_err(|err| self.stop(Step::Preflight, &err))?,
        );
        let fresh = server
            .facts(&[])
            .await
            .map_err(|err| self.stop(Step::Preflight, &err))?;
        facts.current = fresh.current;
        facts.content = fresh.content;
        facts.active = fresh.active;
        facts.systemd = fresh.systemd;
        facts.free_bytes = fresh.free_bytes;
        self.previous.clone_from(&facts.current);
        let commands = &self.config().pipeline.preflight.run;
        if let Err(err) =
            Box::pin(self.remote(&server, Place::AppDir, Step::Preflight, &commands.server)).await
        {
            return Err(self.stop(Step::Preflight, &err));
        }
        if let Err(err) =
            Box::pin(self.local(Step::Preflight, Step::Preflight, None, &commands.local)).await
        {
            return Err(self.stop(Step::Preflight, &err));
        }
        // Files from the deploy folder are there before the build; project
        // files are checked once it has run.
        match files::walk_source(self.deployable, Source::Deploy) {
            Ok(items) => {
                for (path, item) in &items {
                    if let Kind::File { size, .. } = item.kind {
                        self.report.log(
                            LogKind::Success,
                            format!("deploy folder: {path} ({})", human::bytes(size)),
                        );
                    }
                }
            }
            Err(err) => return Err(self.stop(Step::Preflight, &err)),
        }
        self.finish(Step::Preflight, summary);
        Ok((server, facts))
    }

    /// Build: name the release, then run the build commands and check it.
    pub(super) async fn step_build(
        &mut self,
        facts: &Facts,
    ) -> Result<(ReleaseName, String), Ended> {
        self.start(Step::Build, "building on your PC");
        let (name, source) = match build::release_name(self.deployable).await {
            Ok(found) => found,
            Err(err) => return Err(self.stop(Step::Build, &err)),
        };
        let id = name.id();
        self.release = Some(id.clone());
        self.version = Some(name.version.clone());
        let report = self.report;
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
        let build_commands = &self.config().pipeline.build.run.local;
        if build_commands.is_empty() {
            report.log(
                LogKind::Info,
                "pipeline.build.run.local is empty; shipping the files as they are",
            );
        } else if let Err(err) =
            Box::pin(self.local(Step::Build, Step::Build, None, build_commands)).await
        {
            return Err(self.stop(Step::Build, &err.context("the build failed")));
        }
        let built = match check_build(self.deployable, facts, report) {
            Ok(summary) => summary,
            Err(err) => return Err(self.stop(Step::Build, &err)),
        };
        self.finish(
            Step::Build,
            format!("{built} · {}", human::duration(started.elapsed())),
        );
        Ok((name, id))
    }

    /// Pack: gather the files, stop when the live release holds the same
    /// content, then compress.
    pub(super) async fn step_pack(
        &mut self,
        server: &Server,
        name: &ReleaseName,
        id: &str,
        facts: &Facts,
    ) -> Result<(Manifest, Arc<Contents>, Bundle), Ended> {
        self.start(Step::Pack, "packing the release");
        let index = self.at(Step::Pack);
        let deployable = self.deployable;
        let report = self.report;
        let deployer = build::deployer(&deployable.project_dir()).await;
        let mut manifest = Manifest::new(name, &deployer);
        let gathered = async {
            let start_path = start_path(deployable, facts);
            let own = bundle::service_file(deployable, name, start_path.as_deref())?;
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
            Err(err) => return Err(self.stop(Step::Pack, &err)),
        };
        manifest.content = Some(fingerprint);
        // A live release from before fingerprints has none: always a change.
        if let Some(live) = facts.current.as_deref()
            && facts.content == manifest.content
            && !self.deploy_unchanged(live).await
        {
            return Err(self.unchanged(Step::Pack, live));
        }
        let packed = async {
            let on_server = self
                .on_server(server, id, &contents, facts.current.as_deref())
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
            Err(err) => return Err(self.stop(Step::Pack, &err)),
        };
        self.explain(&contents);
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
        Ok((manifest, contents, bundle))
    }

    /// Notes the files the release carries for reasons worth knowing.
    fn explain(&self, contents: &Contents) {
        let report = self.report;
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
    }

    /// Upload the packed release; keep unfinished files for later inspection.
    pub(super) async fn step_upload(
        &self,
        server: &Server,
        id: &str,
        bundle: &Bundle,
    ) -> Result<(), Ended> {
        self.start(Step::Upload, "sending the release");
        let started = Instant::now();
        if let Err(err) = self.send(server, id, bundle, Step::Upload).await {
            self.report.log(
                LogKind::Warn,
                "unfinished upload files were kept for inspection",
            );
            return Err(self.stop(Step::Upload, &err));
        }
        self.finish(
            Step::Upload,
            format!(
                "{} in {}",
                human::bytes(bundle.data.len() as u64),
                human::duration(started.elapsed())
            ),
        );
        Ok(())
    }

    /// Unpack: put the release in `releases/<id>` and link its shared folders.
    pub(super) async fn step_unpack(
        &self,
        server: &Server,
        id: &str,
        contents: &Arc<Contents>,
        manifest: &Manifest,
        bundle: &Bundle,
    ) -> Result<(), Ended> {
        self.start(Step::Unpack, &format!("unpacking into releases/{id}"));
        let shared = &self.config().service.shared;
        let unpacked = async {
            let summary = self.unpack(server, id, contents, manifest, bundle).await?;
            server.link_shared(id, shared, self.report).await?;
            Ok::<_, anyhow::Error>(summary)
        }
        .await;
        let summary = match unpacked {
            Ok(summary) => summary,
            Err(err) => {
                self.report.log(
                    LogKind::Warn,
                    "unfinished release files were kept for inspection",
                );
                return Err(self.stop(Step::Unpack, &err));
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
        Ok(())
    }

    /// Setup: in the new release, while the old one still serves.
    pub(super) async fn step_setup(&self, server: &Server, id: &str) -> Result<(), Ended> {
        if self.has_setup() {
            self.start(Step::Setup, &format!("setting up releases/{id}"));
            match self.set_up(server, id).await {
                Ok(summary) => self.finish(Step::Setup, summary),
                Err(err) => {
                    self.keep_new(id);
                    return Err(self.stop(Step::Setup, &err));
                }
            }
        } else {
            self.skip(Step::Setup, "nothing to run");
        }
        Ok(())
    }

    /// Backup: what stays of the release once its folder goes.
    pub(super) async fn step_backup(
        &self,
        server: &Server,
        id: &str,
        bundle: &Bundle,
    ) -> Result<(), Ended> {
        if self.config().pipeline.backup.keep.compressed == 0 {
            self.skip(Step::Backup, "keep.compressed is 0");
        } else {
            self.start(Step::Backup, &format!("packing backups/{id}.tar.zst"));
            match self.back_up(server, id, &bundle.paths).await {
                Ok(summary) => self.finish(Step::Backup, summary),
                Err(err) => {
                    self.keep_new(id);
                    return Err(self.stop(Step::Backup, &err));
                }
            }
        }
        Ok(())
    }

    /// Switch: point `current` at the new release and install its unit.
    pub(super) async fn step_switch(&self, server: &Server, id: &str) -> Result<(), Ended> {
        self.start(Step::Switch, "making it the current release");
        let switched = async { switch_processes(&[server], id, self.report).await }.await;
        if let Err(err) = switched {
            return Err(self.recover(server, Step::Switch, err).await);
        }
        self.finish(Step::Switch, format!("current → {id}"));
        Ok(())
    }

    /// Restart: bring the service up on the new release.
    pub(super) async fn step_restart(&self, server: &Server) -> Result<(), Ended> {
        self.start(Step::Restart, &self.config().service.name);
        if let Err(err) = restart_processes(&[server], self.report).await {
            return Err(self.recover(server, Step::Restart, err).await);
        }
        self.finish(Step::Restart, "");
        Ok(())
    }

    /// Health: wait for the service and its URL to answer.
    pub(super) async fn step_health(&self, server: &Server) -> Result<(), Ended> {
        self.start(Step::Health, "waiting for the new release to settle");
        match Box::pin(self.health(server)).await {
            Ok(summary) => {
                self.finish(Step::Health, summary);
                Ok(())
            }
            Err(err) => Err(self.recover(server, Step::Health, err).await),
        }
    }
}
