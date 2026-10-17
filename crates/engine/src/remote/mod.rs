//! What shipdeck does on the server, as small `sh` scripts over one session.
//!
//! Layout under `service.dir`:
//! `current -> releases/<id>`, `releases/<id>/`, `backups/<id>.tar.zst`,
//! `shared/`.

mod backup;
mod control;
mod delta;
mod health;
mod pm2;
mod preflight;
mod releases;

pub use backup::{Picked, ReleaseFile, pick};
pub use control::{Control, control_commands, log_command};
pub use delta::Unpacked;
pub use health::{HttpCheck, Probe};
pub use pm2::{PM2_NEWEST_MAJOR, app_state, parse_version};
pub use preflight::Facts;
pub use releases::{Kind, Listing, ReleaseEntry, RotationPlan, Status, plan_rotation};

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Result, bail};

use crate::config::{Manager, Service, release_path};
use crate::deploy::report::{LogKind, Reporter};
use crate::release::is_safe_id;
use crate::ssh::{Output, Session, Stream, sh_quote};

pub struct Server {
    session: Arc<Session>,
    dir: String,
    service: String,
    manager: Manager,
    pm2_app: String,
    /// The PM2 file, relative to the release.
    pm2_file: String,
    /// What the app gets when PM2 starts it.
    env: CommandEnv,
}

impl Server {
    pub fn new(session: Arc<Session>, service: &Service) -> Self {
        Self {
            session,
            dir: service.dir.clone(),
            service: service.name.clone(),
            manager: service.manager,
            pm2_app: service.pm2_app().to_string(),
            pm2_file: service.pm2_file(),
            env: CommandEnv {
                service: service.env.clone(),
                env_file: service.env_file.as_deref().map(release_path),
                vars: Vec::new(),
            },
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn manager(&self) -> Manager {
        self.manager
    }

    fn q_dir(&self) -> String {
        sh_quote(&self.dir)
    }

    fn q_service(&self) -> String {
        sh_quote(&self.service)
    }

    /// Where the bundle is uploaded before it is unpacked.
    pub fn upload_path(&self, id: &str) -> String {
        format!("{}/releases/.{id}.tar.zst", self.dir)
    }

    /// The commands that need root here; see [`sudo_commands`]. PM2 needs
    /// none.
    pub fn sudo_commands(&self) -> Vec<String> {
        match self.manager {
            Manager::Systemd => sudo_commands(&self.dir, &self.service),
            Manager::Pm2 => Vec::new(),
        }
    }

    /// Logged in as root, so nothing needs sudo.
    pub fn is_root(&self) -> bool {
        self.session.user == "root"
    }

    /// `sudo -n `, or nothing for root.
    fn sudo(&self) -> &'static str {
        if self.is_root() { "" } else { "sudo -n " }
    }

    /// Creates `service.dir` for an app's first deploy: as the user if it
    /// may, else with sudo (then handed to the user), else it says so.
    pub async fn create_app_dir(&self, report: &Reporter) -> Result<()> {
        let script = format!(
            "if mkdir -p {dir} 2>/dev/null; then exit 0; fi
if sudo -n mkdir -p {dir} 2>/dev/null && sudo -n chown \"$(id -un):$(id -gn)\" {dir}; then
  echo 'created with sudo'
  exit 0
fi
echo \"$(id -un) may not create {shown}, and has no sudo without a password\" >&2
exit 1",
            dir = self.q_dir(),
            shown = self.dir,
        );
        self.run(&format!("create {}", self.dir), &script, report)
            .await?;
        Ok(())
    }

    /// Runs `script`, logging `what` and every output line, and fails on a
    /// non-zero exit.
    async fn run(&self, what: &str, script: &str, report: &Reporter) -> Result<Output> {
        self.exec_logged(what, script, report).await?.check(what)
    }

    /// Runs `script`, logging `what` and every output line; the caller
    /// reads how it ended.
    async fn exec_logged(&self, what: &str, script: &str, report: &Reporter) -> Result<Output> {
        report.log(LogKind::Command, format!("{} › {what}", self.session.label));
        self.session
            .exec_with(script, |stream, line| {
                let kind = match stream {
                    Stream::Out => LogKind::Out,
                    Stream::Err => LogKind::Err,
                };
                report.log(kind, line);
            })
            .await
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

    /// Removes every trace of a release that did not make it, its backup
    /// and the files sent to unpack it included.
    pub async fn discard(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = discard_script(&self.dir, id);
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
    /// service at boot. PM2 reads its file from `current` at restart.
    pub async fn install_unit(&self, report: &Reporter) -> Result<()> {
        if self.manager == Manager::Pm2 {
            return Ok(());
        }
        let (dir, svc, sudo) = (&self.dir, &self.service, self.sudo());
        let script = format!(
            "set -e
{sudo}install -m 0644 {src} {dst}
{sudo}systemctl daemon-reload
{sudo}systemctl enable {svc_q} 2>&1",
            src = sh_quote(&format!("{dir}/current/{svc}.service")),
            dst = sh_quote(&format!("/etc/systemd/system/{svc}.service")),
            svc_q = self.q_service(),
        );
        self.run(&format!("install {svc}.service"), &script, report)
            .await?;
        Ok(())
    }

    /// Restarts the service on the release `current` points to. PM2 starts
    /// or reloads the app from `current`'s file with the service's
    /// environment, then saves its list so the app comes back after a reboot.
    pub async fn restart(&self, report: &Reporter) -> Result<()> {
        if self.manager == Manager::Pm2 {
            let script = self.pm2_script("startOrReload");
            self.run(&format!("reload {} in pm2", self.pm2_app), &script, report)
                .await?;
            return Ok(());
        }
        let script = format!("{}systemctl restart {}", self.sudo(), self.q_service());
        self.run(&format!("restart {}", self.service), &script, report)
            .await?;
        Ok(())
    }

    /// `pm2 <verb>` on `current`'s PM2 file, for this app only, with the
    /// service's environment loaded; then `pm2 save`, so the app comes back
    /// after a reboot.
    fn pm2_script(&self, verb: &str) -> String {
        let current = format!("{}/current", self.dir);
        let file = sh_quote(&format!("{current}/{}", self.pm2_file));
        let mut script = env_prelude(&current, &self.env, self.env.env_file.clone());
        script.push_str(&format!(
            "[ -f {file} ] || {{ echo {missing} >&2; exit 1; }}
pm2 {verb} {file} --only {app} --update-env
pm2 save
",
            missing = sh_quote(&format!(
                "this release has no {}; was it deployed with systemd?",
                self.pm2_file
            )),
            app = sh_quote(&self.pm2_app),
        ));
        script
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
        let mut script = env_prelude(&dir, env, env_file);
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

    /// The service's last 30 log lines: its journal, or PM2's logs.
    pub async fn journal(&self) -> Result<Vec<String>> {
        let script = match self.manager {
            Manager::Systemd => format!(
                "{}journalctl -u {} -n 30 --no-pager 2>&1",
                self.sudo(),
                self.q_service()
            ),
            Manager::Pm2 => format!(
                "pm2 logs {} --lines 30 --nostream --raw 2>&1",
                sh_quote(&self.pm2_app)
            ),
        };
        let out = self.session.exec(&script).await?;
        Ok(out.stdout.lines().map(str::to_string).collect())
    }
}

/// `set -e`, `cd dir`, then `env` exported: `service.env`, then `env_file`
/// (as systemd reads it), then `vars`.
fn env_prelude(dir: &str, env: &CommandEnv, env_file: Option<String>) -> String {
    let mut script = format!("set -e\ncd {}\n", sh_quote(dir));
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
    script
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

/// Removes `releases/<id>`, everything sent to make it, and its backup.
fn discard_script(dir: &str, id: &str) -> String {
    format!(
        "cd {dir} && rm -rf releases/{id} releases/.{id}.partial releases/.{id}.tar.zst \
releases/.{id}.paths releases/.{id}.sha256 releases/.{id}.checked releases/.{id}.reuse.* \
backups/{id}.tar.zst backups/.{id}.tar.zst.part backups/.{id}.files",
        dir = sh_quote(dir)
    )
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
