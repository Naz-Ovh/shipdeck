//! Bundle transport, shared backups, process transitions and recovery.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, bail};

use super::apps;
use super::multiple::Multiple;
use super::report::LogKind;
use crate::bundle::{self, Bundle, Contents};
use crate::config::monorepo::{AppDefinition, ResolvedApp};
use crate::release::{Manifest, ReleaseName};
use crate::remote::{Facts, Server, Source, Unpacked};

impl Multiple<'_> {
    /// Gathers, fingerprints and compresses one release for every app.
    pub(super) async fn bundle(
        &self,
        resolved: &[ResolvedApp],
        name: &ReleaseName,
        facts: &Facts,
        server: &Server,
    ) -> Result<(Contents, Manifest, Bundle)> {
        let contents = bundle::gather_monorepo(
            &self.config,
            &self.deployment.folder,
            resolved,
            &self.deployment.endpoint.user,
            name,
            facts,
            |_, _| {},
        )?;
        let mut manifest =
            Manifest::new(name, &crate::build::deployer(&self.deployment.folder).await);
        manifest.content = Some(bundle::fingerprint_monorepo(
            &contents,
            &self.config,
            &self.deployment.folder,
            resolved,
            &self.deployment.endpoint.user,
            facts,
        )?);
        let hashes = if self.previous.is_some() {
            match server
                .current_hashes(&name.id(), &contents.candidates())
                .await
            {
                Ok(hashes) => hashes,
                Err(err) => {
                    self.report
                        .log(LogKind::Warn, format!("file reuse unavailable: {err:#}"));
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };
        let bundle = bundle::pack(&contents, &manifest, &hashes, |_, _| {})?;
        let needed = u64::try_from(bundle.data.len())?
            .checked_mul(2)
            .and_then(|n| n.checked_add(bundle.input_bytes))
            .ok_or_else(|| anyhow::anyhow!("release size overflow"))?;
        if facts.free_bytes.is_some_and(|free| free < needed) {
            bail!("not enough free disk space for this release");
        }
        Ok((contents, manifest, bundle))
    }

    /// Uploads the archive and its verification plan.
    pub(super) async fn send(
        &self,
        server: &Server,
        id: &str,
        bundle: &Bundle,
        retry: bool,
    ) -> Result<()> {
        server.prepare(self.report).await?;
        server
            .upload_bundle(id, &bundle.data, retry, |_, _| {})
            .await?;
        server.upload_plan(id, &bundle.receipt, &bundle.reuse).await
    }

    /// Verifies unpacking, retrying with all files when reuse fails.
    pub(super) async fn unpack(
        &self,
        server: &Server,
        id: &str,
        contents: &Contents,
        manifest: &Manifest,
        bundle: &Bundle,
    ) -> Result<()> {
        let modes: Vec<_> = bundle.reuse.keys().copied().collect();
        if matches!(
            server
                .unpack(
                    id,
                    &modes,
                    bundle.input_bytes,
                    u64::try_from(bundle.paths.len())?,
                    self.report
                )
                .await?,
            Unpacked::Verified
        ) {
            return Ok(());
        }
        if bundle.reused == 0 {
            bail!("unpacked files do not match the bundle");
        }
        let whole = bundle::pack(contents, manifest, &HashMap::new(), |_, _| {})?;
        self.send(server, id, &whole, true).await?;
        if !matches!(
            server
                .unpack(
                    id,
                    &[],
                    whole.input_bytes,
                    u64::try_from(whole.paths.len())?,
                    self.report
                )
                .await?,
            Unpacked::Verified
        ) {
            bail!("full release verification failed");
        }
        Ok(())
    }

    /// Links each app's persistent folders into the shared release.
    pub(super) async fn link(&self, server: &Server, id: &str) -> Result<()> {
        let mut shared = Vec::new();
        for (name, definition) in &self.config.apps {
            let AppDefinition::Inline(app) = definition else {
                bail!("unresolved app {name}")
            };
            shared.extend(app.shared.iter().map(|path| format!("apps/{name}/{path}")));
        }
        server.link_shared(id, &shared, self.report).await
    }

    /// Creates one backup using each app's file selection.
    pub(super) async fn backup(
        &self,
        server: &Server,
        id: &str,
        deployed: &[String],
    ) -> Result<()> {
        if self.pipeline.backup.keep.compressed == 0 {
            return Ok(());
        }
        let own: Vec<_> = self
            .config
            .apps
            .iter()
            .filter_map(|(name, definition)| match definition {
                AppDefinition::Inline(app) => {
                    let service = app.service(&self.config, name);
                    service.own_file().or_else(|| {
                        service.pm2.ecosystem.as_ref().map(|file| {
                            format!("apps/{name}/{}", crate::config::release_path(file))
                        })
                    })
                }
                AppDefinition::File(_) => None,
            })
            .collect();
        let picked = crate::remote::pick_apps(
            &self.config,
            &server.release_files(id).await?,
            deployed,
            &own,
        )?;
        for pattern in picked.unmatched {
            self.report.log(
                LogKind::Warn,
                format!("backup pattern matched nothing: {pattern}"),
            );
        }
        server.back_up(id, &picked.paths, self.report).await?;
        Ok(())
    }

    /// Applies retention once to the shared deployment.
    pub(super) async fn rotate(&self, server: &Server) -> Result<()> {
        let plan = crate::remote::plan_rotation(&server.list().await?, self.pipeline.backup.keep);
        for id in plan.close.iter().chain(&plan.drop_open) {
            server.drop_open(id, self.report).await?;
        }
        for id in &plan.drop_backup {
            server.drop_backup(id, self.report).await?;
        }
        Ok(())
    }

    /// Checks both sides of a transition before changing any live process.
    pub(super) async fn check_permissions(&self, session: &Arc<crate::ssh::Session>) -> Result<()> {
        let mut servers = apps::servers(&self.config, session)?;
        if let Some(previous) = &self.previous {
            let first = servers.first().ok_or_else(|| anyhow::anyhow!("no apps"))?;
            let old = apps::saved(first, previous, Source::Folder).await?;
            servers.extend(apps::servers(&old, session)?);
        }
        let mut tools = std::collections::BTreeSet::from([
            "tar".to_string(),
            "zstd".to_string(),
            "sha256sum".to_string(),
        ]);
        tools.extend(self.pipeline.preflight.requires.iter().cloned());
        let mut http = self.pipeline.health.url.is_some();
        for (name, definition) in &self.config.apps {
            let AppDefinition::Inline(app) = definition else {
                bail!("unresolved app {name}")
            };
            let pipeline = app.pipeline(&self.config.pipeline)?;
            tools.extend(pipeline.preflight.requires);
            http |= pipeline.health.url.is_some();
            let service = app.service(&self.config, name);
            if !service.is_static() {
                if service.manager == crate::config::Manager::Pm2 {
                    tools.insert("pm2".to_string());
                }
                if service.pm2.ecosystem.is_none()
                    && let (
                        crate::config::Program::Tool(tool) | crate::config::Program::Absolute(tool),
                        _,
                    ) = service.command(None)
                {
                    tools.insert(tool);
                }
            }
        }
        if http {
            tools.insert("curl".to_string());
            tools.insert("wget".to_string());
        }
        let tools: Vec<_> = tools.into_iter().collect();
        for server in servers {
            let facts = server.facts(&tools).await?;
            if !facts.sudo_missing.is_empty() {
                bail!("sudo does not allow {}", facts.sudo_missing.join(", "));
            }
            for (name, path) in &facts.tools {
                if path.is_none() && !(http && matches!(name.as_str(), "curl" | "wget")) {
                    bail!("server is missing {name}");
                }
            }
            if http && facts.tool_path("curl").is_none() && facts.tool_path("wget").is_none() {
                bail!("HTTP health checks require curl or wget on the server");
            }
            server.check_stop_permission().await?;
        }
        Ok(())
    }

    /// Stops removed or replaced processes before one shared switch.
    pub(super) async fn transition(&self, session: &Arc<crate::ssh::Session>) -> Result<()> {
        let servers = apps::servers(&self.config, session)?;
        let first = servers.first().ok_or_else(|| anyhow::anyhow!("no apps"))?;
        if let Some(previous) = &self.previous {
            let old = apps::saved(first, previous, Source::Folder).await?;
            for (name, definition) in &old.apps {
                let AppDefinition::Inline(old_app) = definition else {
                    bail!("unresolved app {name}")
                };
                let old_service = old_app.service(&old, name);
                let same = match self.config.apps.get(name) {
                    Some(AppDefinition::Inline(app)) => {
                        let service = app.service(&self.config, name);
                        old_service.name == service.name
                            && old_service.manager == service.manager
                            && old_service.pm2_app() == service.pm2_app()
                    }
                    _ => false,
                };
                if !same {
                    Server::for_app(Arc::clone(session), &old_service, name)?
                        .retire(self.report)
                        .await?;
                }
            }
        }
        let id = self
            .release
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no release"))?;
        super::switch_processes(&servers.iter().collect::<Vec<_>>(), id, self.report).await
    }
}
