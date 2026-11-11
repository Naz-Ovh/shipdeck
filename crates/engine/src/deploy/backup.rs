//! Step 5: what stays on the server once a release's folder goes.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};

use super::report::LogKind;
use super::run::{Ended, Run};
use super::steps::Step;
use crate::bundle::Plan;
use crate::config::count;
use crate::config::glob::{Rules, Verdict};
use crate::human;
use crate::remote::{ReleaseFile, Server};

impl Run<'_> {
    /// Backup: your commands, then `backups/<id>.tar.zst`.
    pub(super) async fn step_backup(&mut self, server: &Server) -> Result<(), Ended> {
        self.start(Step::Backup, "packing the backup");
        let backed = Box::pin(self.backup(server)).await;
        match backed {
            Ok(detail) => {
                self.finish(Step::Backup, detail);
                Ok(())
            }
            Err(err) => Err(self.stop(Step::Backup, &err)),
        }
    }

    async fn backup(&mut self, server: &Server) -> Result<String> {
        Box::pin(self.exec(Step::Backup).step(Step::Backup, None)).await?;
        self.check_lock()?;
        let id = self
            .release
            .clone()
            .ok_or_else(|| anyhow!("the release is not named"))?;
        let keep = self.deployment.keep();
        if keep.compressed == 0 {
            return Ok("keep.compressed is 0, so no backup is made".into());
        }
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| anyhow!("the release has no files"))?;
        let listing = server.release_files(&id).await?;
        let picked = select(self, &plan, &listing);
        for pattern in &picked.unmatched {
            self.report.log(
                LogKind::Warn,
                format!("backup pattern matched nothing: {pattern}"),
            );
        }
        let bytes = server.back_up(&id, &picked.paths, self.report).await?;
        Ok(format!(
            "{} · {}",
            count(picked.paths.len(), "path"),
            human::bytes(bytes)
        ))
    }
}

/// What goes into one release's backup.
#[derive(Debug, Default)]
struct Picked {
    paths: Vec<String>,
    unmatched: Vec<String>,
}

/// Uploaded files and `.shipdeck/`, plus `include`, minus `exclude`, per
/// release and per app. Shared folders never.
fn select(run: &Run<'_>, plan: &Plan, listing: &[ReleaseFile]) -> Picked {
    let link_paths: Vec<&str> = plan.links.iter().map(|link| link.path.as_str()).collect();
    let deployed: BTreeSet<&str> = plan
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let release = &run.deployment.steps().backup;
    let release_rules = Rules::parse(&release.include, &release.exclude).unwrap_or_default();
    let mut apps: BTreeMap<&str, Rules> = BTreeMap::new();
    for (name, app) in &run.deployment.deploy.apps {
        if let Ok(rules) = Rules::parse(&app.steps.backup.include, &app.steps.backup.exclude) {
            apps.insert(name.as_str(), rules);
        }
    }
    let mut hits: BTreeMap<&str, usize> = BTreeMap::new();
    for (name, rules) in
        std::iter::once(("", &release_rules)).chain(apps.iter().map(|(name, rules)| (*name, rules)))
    {
        for _ in 0..rules.include.len() {
            hits.entry(name).or_insert(0);
        }
    }
    let mut release_hits = vec![0; release_rules.include.len()];
    let mut app_hits: BTreeMap<&str, Vec<usize>> = apps
        .iter()
        .map(|(name, rules)| (*name, vec![0; rules.include.len()]))
        .collect();
    let mut folders: BTreeMap<String, Verdict> = BTreeMap::new();
    let mut paths = Vec::new();
    let mut sorted: Vec<&ReleaseFile> = listing.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    for file in sorted {
        let path = file.path.as_str();
        if link_paths
            .iter()
            .any(|link| path == *link || path.starts_with(&format!("{link}/")))
        {
            continue;
        }
        let (scope, relative, rules, hits_for) = if let Some((name, rest)) = path
            .strip_prefix("apps/")
            .and_then(|rest| rest.split_once('/'))
            && let Some(rules) = apps.get(name)
        {
            (
                name.to_string(),
                rest,
                rules,
                app_hits.get_mut(name).map(std::vec::Vec::as_mut_slice),
            )
        } else {
            (
                String::new(),
                path,
                &release_rules,
                Some(release_hits.as_mut_slice()),
            )
        };
        let parent = relative
            .rsplit_once('/')
            .and_then(|(parent, _)| folders.get(&format!("{scope}/{parent}")))
            .copied()
            .unwrap_or_default();
        let verdict = match hits_for {
            Some(hits) => rules.judge(parent, relative, file.is_dir, hits),
            None => parent,
        };
        if file.is_dir {
            folders.insert(format!("{scope}/{relative}"), verdict);
        }
        let generated = path == ".shipdeck" || path.starts_with(".shipdeck/");
        let wanted = generated
            || deployed.contains(path)
            || verdict.included()
            || (file.is_dir && has_wanted_below(plan, path));
        if wanted && !verdict.excluded() {
            paths.push(file.path.clone());
        }
    }
    let mut unmatched: Vec<String> = release_rules
        .include
        .iter()
        .zip(&release_hits)
        .filter(|(pattern, hits)| !pattern.negated && **hits == 0)
        .map(|(pattern, _)| pattern.text.clone())
        .collect();
    for (name, rules) in &apps {
        let hits = app_hits.get(name).cloned().unwrap_or_default();
        unmatched.extend(
            rules
                .include
                .iter()
                .zip(&hits)
                .filter(|(pattern, hits)| !pattern.negated && **hits == 0)
                .map(|(pattern, _)| format!("{name}: {}", pattern.text)),
        );
    }
    let _ = &mut hits;
    Picked { paths, unmatched }
}

/// Whether a folder holds a path that was uploaded.
fn has_wanted_below(plan: &Plan, folder: &str) -> bool {
    let prefix = format!("{folder}/");
    plan.entries
        .iter()
        .any(|entry| entry.path.starts_with(&prefix))
}
