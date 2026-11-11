//! Step 3: pack the files, send them, unpack them and link the shared
//! folders.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Result, anyhow, bail};

use super::build::Throttle;
use super::report::{LogKind, Unit};
use super::run::{Ended, Run};
use super::steps::Step;
use crate::bundle::{self, Plan};
use crate::config::count;
use crate::human;
use crate::remote::{Facts, Server, Unpacked};

impl Run<'_> {
    /// Upload: pack, send, unpack, link.
    pub(super) async fn step_upload(
        &mut self,
        server: &Server,
        facts: &Facts,
    ) -> Result<(), Ended> {
        self.start(Step::Upload, "sending the release");
        let uploaded = Box::pin(self.upload(server, facts)).await;
        match uploaded {
            Ok(detail) => {
                self.finish(Step::Upload, detail);
                Ok(())
            }
            Err(err) => Err(self.stop(Step::Upload, &err)),
        }
    }

    async fn upload(&mut self, server: &Server, facts: &Facts) -> Result<String> {
        let exec = self.exec(Step::Upload);
        Box::pin(exec.step(Step::Upload, None)).await?;
        self.check_lock()?;
        let id = self
            .release
            .clone()
            .ok_or_else(|| anyhow!("the release is not named"))?;
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| anyhow!("the release has no files"))?;
        if server
            .list()
            .await?
            .entries
            .iter()
            .any(|entry| entry.id == id)
        {
            bail!("release {id} already exists; wait one second before deploying again");
        }
        server.prepare(self.report).await?;
        self.check_lock()?;
        let on_server = if self.previous.is_some() {
            match server.current_hashes(&id, &plan.candidates()).await {
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
        let bundle = self.pack(&plan, &on_server, Step::Upload)?;
        if let Some(free) = facts.free_bytes {
            let needed = u64::try_from(bundle.data.len())?
                .checked_mul(2)
                .and_then(|bytes| bytes.checked_add(bundle.input_bytes))
                .ok_or_else(|| anyhow!("release size overflow"))?;
            if free < needed {
                bail!(
                    "the server has {} free; this release needs about {}",
                    human::bytes(free),
                    human::bytes(needed)
                );
            }
        }
        self.send(server, &id, &bundle).await?;
        self.check_lock()?;
        self.open(server, &id, &plan, &bundle).await?;
        self.check_lock()?;
        server
            .link_shared(
                &id,
                &plan
                    .links
                    .iter()
                    .map(|link| link.path.clone())
                    .collect::<Vec<_>>(),
                self.report,
            )
            .await?;
        let sizes = format!(
            "{} -> {}",
            human::bytes(bundle.input_bytes),
            human::bytes(bundle.data.len() as u64)
        );
        Ok(if bundle.reused == 0 {
            format!("{} · {sizes}", count(bundle.files, "file"))
        } else {
            format!(
                "{} · {} already on the server · {} to send · {sizes}",
                count(bundle.files, "file"),
                bundle.reused,
                bundle.sent()
            )
        })
    }

    /// Compresses the release; progress goes to the step.
    fn pack(
        &self,
        plan: &Plan,
        on_server: &HashMap<String, String>,
        row: Step,
    ) -> Result<bundle::Bundle> {
        let index = self.at(row);
        let mut throttle = Throttle::default();
        bundle::pack(plan, on_server, |done, total| {
            if throttle.ready(done, total) {
                self.report
                    .progress(index, done, total, Unit::Bytes, "compressing");
            }
        })
    }

    /// Uploads the archive and its verification plan.
    async fn send(&self, server: &Server, id: &str, bundle: &bundle::Bundle) -> Result<()> {
        let index = self.at(Step::Upload);
        let started = Instant::now();
        server.prepare(self.report).await?;
        self.report
            .log(LogKind::Info, "folders ready; starting the upload");
        report_upload(
            server,
            id,
            bundle,
            false,
            |sent, total, report| {
                let elapsed = started.elapsed().as_nanos().max(1_000_000);
                let speed =
                    u64::try_from(u128::from(sent) * 1_000_000_000 / elapsed).unwrap_or(u64::MAX);
                report.progress(
                    index,
                    sent,
                    total,
                    Unit::Bytes,
                    format!("{}/s", human::bytes(speed)),
                );
            },
            self.report,
        )
        .await?;
        server.upload_plan(id, &bundle.receipt, &bundle.reuse).await
    }

    /// Unpacks and verifies; a mismatch retries with the whole release.
    async fn open(
        &self,
        server: &Server,
        id: &str,
        plan: &Plan,
        bundle: &bundle::Bundle,
    ) -> Result<()> {
        let modes: Vec<u32> = bundle.reuse.keys().copied().collect();
        let count = u64::try_from(bundle.paths.len())?;
        match server
            .unpack(id, &modes, bundle.input_bytes, count, self.report)
            .await?
        {
            Unpacked::Verified => return Ok(()),
            Unpacked::Mismatch(names) if bundle.reused == 0 => {
                bail!(
                    "releases/{id} does not match what was packed: {}",
                    names.join(", ")
                );
            }
            Unpacked::Mismatch(_) => {}
        }
        self.report.log(
            LogKind::Warn,
            "copied files from current did not match; sending the whole release",
        );
        let whole = self.pack(plan, &HashMap::new(), Step::Upload)?;
        server
            .upload_bundle(id, &whole.data, true, |_, _| {})
            .await?;
        server.upload_plan(id, &whole.receipt, &whole.reuse).await?;
        match server
            .unpack(id, &[], whole.input_bytes, count, self.report)
            .await?
        {
            Unpacked::Verified => Ok(()),
            Unpacked::Mismatch(names) => bail!(
                "releases/{id} does not match what was packed, even sent whole: {}",
                names.join(", ")
            ),
        }
    }
}

/// Sends the archive with progress, through a small adapter.
async fn report_upload(
    server: &Server,
    id: &str,
    bundle: &bundle::Bundle,
    retry: bool,
    mut progress: impl FnMut(u64, u64, &super::report::Reporter),
    report: &super::report::Reporter,
) -> Result<()> {
    server
        .upload_bundle(id, &bundle.data, retry, |sent, total| {
            progress(sent, total, report);
        })
        .await
}
