//! Setup, the backup and the rotation of old releases on the server.

use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

use anyhow::Result;

use super::report::{LogKind, Reporter, Unit};
use super::{Run, Step, count};
use crate::config::{Deployable, release_path};
use crate::human;
use crate::release::MANIFEST_FILE;
use crate::remote::{Place, Server, pick, plan_rotation};

impl<'a> Run<'a> {
    /// Whether `pipeline.setup` has anything to run.
    pub(super) fn has_setup(&self) -> bool {
        let setup = &self.config().pipeline.setup;
        !(setup.run.server.is_empty() && setup.check.server.is_empty())
    }

    /// Runs `pipeline.setup` in `releases/<id>`: `run.server`, then
    /// `check.server`. Returns the step's summary.
    pub(super) async fn set_up(&self, server: &Server, id: &str) -> Result<String> {
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
    pub(super) async fn back_up(
        &self,
        server: &Server,
        id: &str,
        deployed: &[String],
    ) -> Result<String> {
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
    pub(super) async fn remove_new(&self, server: &Server, id: &str) {
        let _ = server.discard(id, self.report).await;
        let live = match &self.previous {
            Some(previous) => format!("; {previous} is still live"),
            None => String::new(),
        };
        self.report
            .log(LogKind::Info, format!("releases/{id} was removed{live}"));
    }
}

/// Points `current` back at `id` and reinstalls its unit.
pub(super) async fn restore(server: &Server, id: &str, report: &Reporter) -> Result<()> {
    server.switch(id, report).await?;
    server.install_unit(report).await
}

pub(super) async fn rotate(
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
