//! What shipdeck does on the server, as small `sh` scripts over one session.
//!
//! Layout under `service.dir`:
//! `current -> releases/<id>`, `releases/<id>/`, `backups/<id>.tar.gz`, `shared/`.

mod health;
mod preflight;
mod releases;

pub use health::{HttpCheck, Probe};
pub use preflight::Facts;
pub use releases::{Kind, Listing, ReleaseEntry, RotationPlan, Status, plan_rotation};

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Result, bail};

use crate::config::{Service, release_path};
use crate::deploy::report::{LogKind, Reporter};
use crate::release::is_safe_id;
use crate::ssh::{Output, Session, Stream, sh_quote};

pub struct Server {
    session: Arc<Session>,
    dir: String,
    service: String,
}

impl Server {
    pub fn new(session: Arc<Session>, service: &Service) -> Self {
        Self {
            session,
            dir: service.dir.clone(),
            service: service.name.clone(),
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

    /// The commands that need root here; see [`sudo_commands`].
    pub fn sudo_commands(&self) -> Vec<String> {
        sudo_commands(&self.dir, &self.service)
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

    /// Links each `service.shared` folder into `releases/<id>`, creating it
    /// in `shared/` the first time.
    pub async fn link_shared(&self, id: &str, shared: &[String], report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        if shared.is_empty() {
            return Ok(());
        }
        let mut script = format!("set -e\ncd {}\n", self.q_dir());
        for entry in shared {
            let entry = release_path(entry);
            let link = sh_quote(&format!("releases/{id}/{entry}"));
            script.push_str(&format!(
                "mkdir -p {shared}
if [ -e {link} ] || [ -L {link} ]; then echo {exists} >&2; exit 1; fi
mkdir -p \"$(dirname {link})\"
ln -s {target} {link}
echo {done}
",
                shared = sh_quote(&format!("shared/{entry}")),
                target = sh_quote(&format!("{}/shared/{entry}", self.dir)),
                exists = sh_quote(&format!(
                    "releases/{id}/{entry} came in the release, but it is shared"
                )),
                done = sh_quote(&format!("{entry} -> shared/{entry}")),
            ));
        }
        self.run("link service.shared into the release", &script, report)
            .await?;
        Ok(())
    }

    /// Runs one command from deploy.yaml at `place`, streaming its output.
    /// It sees `service.env`, then `service.env_file` (as systemd reads it:
    /// no shell expansion), then `vars`.
    pub async fn command(
        &self,
        place: &Place,
        command: &str,
        env: &CommandEnv,
        report: &Reporter,
    ) -> Result<()> {
        let (dir, shown, env_file) = match place {
            Place::AppDir => (
                self.dir.clone(),
                "in service.dir".to_string(),
                env.env_file.as_ref().map(|f| format!("current/{f}")),
            ),
            Place::Release(id) => {
                ensure_id(id)?;
                (
                    format!("{}/releases/{id}", self.dir),
                    format!("in releases/{id}"),
                    env.env_file.clone(),
                )
            }
            Place::Current => (
                format!("{}/current", self.dir),
                "in current".to_string(),
                env.env_file.clone(),
            ),
        };
        let mut script = format!("set -e\ncd {}\n", sh_quote(&dir));
        for (key, value) in &env.service {
            script.push_str(&format!("export {key}={}\n", sh_quote(value)));
        }
        if let Some(file) = env_file {
            script.push_str(ENV_FILE_READER);
            script.push_str(&format!("shipdeck_env {}\n", sh_quote(&file)));
        }
        for (key, value) in &env.vars {
            script.push_str(&format!("export {key}={}\n", sh_quote(value)));
        }
        // The command is shell by design: it comes from deploy.yaml, written
        // by whoever deploys, and runs as the deploy user.
        script.push_str(command);
        script.push('\n');
        report.log(
            LogKind::Command,
            format!("{} › {}  ({shown})", self.session.label, command.trim()),
        );
        let out = self
            .session
            .exec_with(&script, |stream, line| {
                let kind = match stream {
                    Stream::Out => LogKind::Out,
                    Stream::Err => LogKind::Err,
                };
                report.log(kind, line);
            })
            .await?;
        if !out.success() {
            bail!(
                "`{}` exited with code {} on the server",
                command.trim(),
                out.code
            );
        }
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

/// Where a command from deploy.yaml runs on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `service.dir`, for preflight; the env file is the live release's.
    AppDir,
    /// `releases/<id>`, for setup, while the old release is still live.
    Release(String),
    /// `current`, for health and done.
    Current,
}

/// What a server command sees besides the deploy user's environment.
#[derive(Debug, Clone, Default)]
pub struct CommandEnv {
    /// `service.env`.
    pub service: BTreeMap<String, String>,
    /// `service.env_file`, relative to the release.
    pub env_file: Option<String>,
    /// `SHIPDECK_*`.
    pub vars: Vec<(&'static str, String)>,
}

/// Exports the `KEY=value` lines of a file the way systemd's
/// `EnvironmentFile=` reads them: comments and blank lines skipped, an
/// optional `export`, one pair of surrounding quotes removed, nothing
/// expanded. A missing file is skipped.
const ENV_FILE_READER: &str = r#"shipdeck_env() {
  [ -f "$1" ] || return 0
  cr=$(printf '\r')
  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%"$cr"}
    line=${line#"${line%%[![:space:]]*}"}
    case $line in ''|'#'*|';'*) continue ;; esac
    line=${line#export }
    key=${line%%=*}
    [ "$key" = "$line" ] && continue
    value=${line#*=}
    case $key in ''|[0-9]*|*[!A-Za-z0-9_]*) continue ;; esac
    case $value in
      \"*\") value=${value#\"}; value=${value%\"} ;;
      \'*\') value=${value#\'}; value=${value%\'} ;;
    esac
    export "$key=$value"
  done < "$1"
}
"#;

/// The commands that need root, without `sudo`, for an app in `dir` run as
/// `service`. The sudoers rule must allow exactly these; changing one breaks
/// every server set up before.
pub fn sudo_commands(dir: &str, service: &str) -> Vec<String> {
    let svc = service;
    vec![
        "systemctl daemon-reload".into(),
        format!("systemctl enable {svc}"),
        format!("systemctl restart {svc}"),
        format!("install -m 0644 {dir}/current/{svc}.service /etc/systemd/system/{svc}.service"),
        format!("journalctl -u {svc} -n 30 --no-pager"),
    ]
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
