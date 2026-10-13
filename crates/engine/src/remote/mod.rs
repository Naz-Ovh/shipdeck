//! What shipdeck does on the server, as small `sh` scripts over one session.
//!
//! Layout under `remote.dir`:
//! `current -> releases/<id>`, `releases/<id>/`, `backups/<id>.tar.gz`, `shared/`.

mod health;
mod preflight;
mod releases;

pub use health::{HttpCheck, Probe};
pub use preflight::Facts;
pub use releases::{Kind, Listing, ReleaseEntry, RotationPlan, Status, plan_rotation};

use std::sync::Arc;

use anyhow::{Result, bail};

use crate::config::RemoteConfig;
use crate::deploy::report::{LogKind, Reporter};
use crate::release::is_safe_id;
use crate::ssh::{Output, Session, Stream, sh_quote};

pub struct Server {
    session: Arc<Session>,
    dir: String,
    service: String,
}

impl Server {
    pub fn new(session: Arc<Session>, remote: &RemoteConfig) -> Self {
        Self {
            session,
            dir: remote.dir.clone(),
            service: remote.service.clone(),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    fn q_dir(&self) -> String {
        sh_quote(&self.dir)
    }

    fn q_service(&self) -> String {
        sh_quote(&self.service)
    }

    /// Where the bundle is uploaded before it is unpacked.
    pub fn upload_path(&self, id: &str) -> String {
        format!("{}/releases/.{id}.tar.gz", self.dir)
    }

    /// The commands that need root, without `sudo`. The sudoers rule must
    /// allow exactly these.
    pub fn sudo_commands(&self) -> Vec<String> {
        let (dir, svc) = (&self.dir, &self.service);
        vec![
            "systemctl daemon-reload".into(),
            format!("systemctl enable {svc}"),
            format!("systemctl restart {svc}"),
            format!(
                "install -m 0644 {dir}/current/{svc}.service /etc/systemd/system/{svc}.service"
            ),
            format!("journalctl -u {svc} -n 30 --no-pager"),
        ]
    }

    /// Runs `script`, logging `what` and every output line, and fails on a
    /// non-zero exit.
    async fn run(&self, what: &str, script: &str, report: &Reporter) -> Result<Output> {
        report.log(LogKind::Command, format!("{} › {what}", self.session.label));
        let out = self
            .session
            .exec_with(script, |stream, line| {
                let kind = match stream {
                    Stream::Out => LogKind::Out,
                    Stream::Err => LogKind::Err,
                };
                report.log(kind, line);
            })
            .await?;
        out.check(what)
    }

    /// Creates `releases/`, `backups/` and `shared/` if they are missing.
    pub async fn prepare(&self, report: &Reporter) -> Result<()> {
        let dir = self.q_dir();
        self.run(
            "create releases/, backups/ and shared/",
            &format!("cd {dir} && mkdir -p releases backups shared"),
            report,
        )
        .await?;
        Ok(())
    }

    /// Unpacks the uploaded bundle into `releases/<id>/`, through a hidden
    /// folder so a half-written release never looks complete.
    pub async fn unpack(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!(
            "set -e
cd {dir}/releases
if [ -e {id} ]; then echo 'releases/{id} already exists' >&2; exit 1; fi
rm -rf .{id}.partial
mkdir .{id}.partial
tar -xzpf .{id}.tar.gz -C .{id}.partial
rm -f .{id}.tar.gz
mv .{id}.partial {id}
echo \"unpacked $(find {id} -type f | wc -l) files into releases/{id}\"",
            dir = self.q_dir(),
        );
        self.run(&format!("unpack into releases/{id}"), &script, report)
            .await?;
        Ok(())
    }

    /// Removes every trace of a release that did not make it.
    pub async fn discard(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!(
            "cd {dir}/releases && rm -rf {id} .{id}.partial .{id}.tar.gz",
            dir = self.q_dir()
        );
        self.run(&format!("remove releases/{id}"), &script, report)
            .await?;
        Ok(())
    }

    /// Points `current` at `releases/<id>` in one rename, so it is never
    /// missing or half-written.
    pub async fn switch(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!(
            "set -e
cd {dir}
[ -d releases/{id} ] || {{ echo 'releases/{id} does not exist' >&2; exit 1; }}
ln -sfn releases/{id} .current.next
mv -Tf .current.next current
echo \"current -> $(readlink current)\"",
            dir = self.q_dir()
        );
        self.run(&format!("point current at {id}"), &script, report)
            .await?;
        Ok(())
    }

    /// Installs the unit `current` carries, reloads systemd and enables the
    /// service at boot.
    pub async fn install_unit(&self, report: &Reporter) -> Result<()> {
        let (dir, svc) = (&self.dir, &self.service);
        let script = format!(
            "set -e
sudo -n install -m 0644 {src} {dst}
sudo -n systemctl daemon-reload
sudo -n systemctl enable {svc_q} 2>&1",
            src = sh_quote(&format!("{dir}/current/{svc}.service")),
            dst = sh_quote(&format!("/etc/systemd/system/{svc}.service")),
            svc_q = self.q_service(),
        );
        self.run(&format!("install {svc}.service"), &script, report)
            .await?;
        Ok(())
    }

    pub async fn restart(&self, report: &Reporter) -> Result<()> {
        let script = format!("sudo -n systemctl restart {}", self.q_service());
        self.run(&format!("restart {}", self.service), &script, report)
            .await?;
        Ok(())
    }

    /// The service's last 30 journal lines.
    pub async fn journal(&self) -> Result<Vec<String>> {
        let out = self
            .session
            .exec(&format!(
                "sudo -n journalctl -u {} -n 30 --no-pager 2>&1",
                self.q_service()
            ))
            .await?;
        Ok(out.stdout.lines().map(str::to_string).collect())
    }
}

fn ensure_id(id: &str) -> Result<()> {
    if !is_safe_id(id) {
        bail!("refusing an unexpected release name: {id:?}");
    }
    Ok(())
}

/// `key=value` lines from a script, in order.
fn pairs(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .collect()
}
