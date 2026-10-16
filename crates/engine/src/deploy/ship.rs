//! Packing the files, sending only what changed, and unpacking on the server.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow, bail};

use super::report::{LogKind, Unit};
use super::{Run, Step, Throttle, count, some};
use crate::bundle;
use crate::human;
use crate::release::Manifest;
use crate::remote::{Server, Unpacked};

impl<'a> Run<'a> {
    /// What `current` already has of `contents`, hashed by the server just
    /// now. Nothing, with a note why, when nothing is live or the server
    /// cannot say: then every file is sent.
    pub(super) async fn on_server(
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
    pub(super) async fn send(
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
    pub(super) async fn unpack(
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
    pub(super) async fn pack(
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
}
