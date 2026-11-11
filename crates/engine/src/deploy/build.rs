//! Step 2: name the release, run your build, and collect and hash the files.

use anyhow::{Result, bail};

use super::report::{LogKind, Unit};
use super::run::{Ended, Run};
use super::steps::Step;
use crate::build;
use crate::bundle::{self, MANIFEST_FILE};
use crate::human;
use crate::release::{Manifest, ReleaseName};
use crate::remote::Facts;

impl Run<'_> {
    /// Build: name the release, run the build commands, collect the files.
    pub(super) async fn step_build(
        &mut self,
        facts: &Facts,
    ) -> Result<(ReleaseName, String), Ended> {
        self.start(Step::Build, "building on your PC");
        let built = Box::pin(self.build(facts)).await;
        match built {
            Ok((name, id, detail)) => {
                self.finish(Step::Build, detail);
                Ok((name, id))
            }
            Err(BuildEnd::Ended(ended)) => Err(ended),
            Err(BuildEnd::Failed(err)) => Err(self.stop(Step::Build, &err)),
        }
    }

    async fn build(&mut self, facts: &Facts) -> Result<(ReleaseName, String, String), BuildEnd> {
        let (name, source) = build::release_name(self.deployment)
            .await
            .map_err(BuildEnd::from)?;
        let id = name.id();
        self.release = Some(id.clone());
        self.version = Some(name.version.clone());
        self.report.release(&id);
        self.report.log(
            LogKind::Info,
            format!("release {id} (version from {source})"),
        );
        if name.dirty {
            self.report.log(
                LogKind::Warn,
                "the source has uncommitted changes; the release is marked dirty",
            );
        }
        Box::pin(self.exec(Step::Build).step(Step::Build, None))
            .await
            .map_err(BuildEnd::Failed)?;
        self.check_lock().map_err(BuildEnd::Failed)?;
        let deployer = build::deployer(&self.deployment.project).await;
        let mut manifest = Manifest::new(&name, &deployer);
        let settings = self.deployment.settings().map_err(BuildEnd::from)?;
        let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(BuildEnd::from)?;
        let mut plan = bundle::release_plan(
            self.deployment,
            &name,
            &manifest_json,
            &settings,
            &self.start_paths,
        )
        .map_err(BuildEnd::from)?;
        if !plan.missing.is_empty() {
            return Err(BuildEnd::Failed(anyhow::anyhow!(
                "these file patterns matched nothing: {}",
                plan.missing.join(", ")
            )));
        }
        self.check_binaries(facts)?;
        let index = self.at(Step::Build);
        let mut throttle = Throttle::default();
        bundle::hash(&mut plan, |done, total| {
            if throttle.ready(done, total) {
                self.report
                    .progress(index, done, total, Unit::Bytes, "hashing");
            }
        })
        .map_err(BuildEnd::from)?;
        let fingerprint = bundle::fingerprint(&plan);
        manifest.content = Some(fingerprint.clone());
        let json = serde_json::to_vec_pretty(&manifest).map_err(BuildEnd::from)?;
        plan.set_bytes(MANIFEST_FILE, json)
            .map_err(BuildEnd::from)?;
        if let Some(live) = facts.current.as_deref()
            && facts.content.as_deref() == Some(fingerprint.as_str())
            && !self.report.ask_unchanged(&id, live).await.unwrap_or(false)
        {
            self.report.skip(self.at(Step::Build), "nothing changed");
            self.skip_rest(Step::Build, "nothing changed");
            return Err(BuildEnd::Ended(Ended {
                outcome: super::report::Outcome::Unchanged {
                    live: live.to_string(),
                },
                failed: None,
            }));
        }
        let files = plan
            .entries
            .iter()
            .filter(|entry| !matches!(entry.source, bundle::Source::Link(_)))
            .count();
        let detail = format!("{} · {}", crate::config::count(files, "file"), name.tag());
        self.plan = Some(plan);
        Ok((name, id, detail))
    }

    /// Every binary can run on the server.
    fn check_binaries(&self, facts: &Facts) -> Result<(), BuildEnd> {
        for name in self.deployment.deploy.apps.keys() {
            let Some(path) = self.deployment.binary_local(name) else {
                continue;
            };
            check_binary(&path, facts, self.report).map_err(BuildEnd::Failed)?;
        }
        Ok(())
    }
}

/// A build's ending: a failure that stops the deploy, or an unchanged
/// release that ends it.
enum BuildEnd {
    Failed(anyhow::Error),
    Ended(Ended),
}

impl From<serde_json::Error> for BuildEnd {
    fn from(err: serde_json::Error) -> Self {
        Self::Failed(err.into())
    }
}

impl From<anyhow::Error> for BuildEnd {
    fn from(err: anyhow::Error) -> Self {
        Self::Failed(err)
    }
}

/// Checks a binary can run on the server.
pub(super) fn check_binary(
    binary: &std::path::Path,
    facts: &Facts,
    report: &super::report::Reporter,
) -> Result<()> {
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
        return Ok(());
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
        Some(_) => Ok(()),
    }
}

/// Lets a progress update through at most every 1% or 100 ms.
#[derive(Default)]
pub(crate) struct Throttle {
    last_percent: Option<u64>,
    last_at: Option<std::time::Instant>,
}

impl Throttle {
    pub(crate) fn ready(&mut self, done: u64, total: u64) -> bool {
        let percent = (done * 100).checked_div(total).unwrap_or(100);
        let now = std::time::Instant::now();
        let due = self
            .last_at
            .is_none_or(|at| now.duration_since(at) >= std::time::Duration::from_millis(100));
        if done >= total || self.last_percent != Some(percent) || due {
            self.last_percent = Some(percent);
            self.last_at = Some(now);
            true
        } else {
            false
        }
    }
}
