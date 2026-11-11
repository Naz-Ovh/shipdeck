//! What shipdeck does on the server, as small `sh` scripts over one session.
//!
//! Layout under `dir`:
//! `current -> releases/<id>`, `releases/<id>/`, `backups/<id>.tar.zst`,
//! `shared/`. Shipdeck's own files live in each release's `.shipdeck/`.

mod backup;
mod control;
mod delta;
mod health;
mod inspect;
mod lease;
mod pm2;
mod preflight;
mod releases;

pub use backup::ReleaseFile;
pub use control::{Control, control_commands, log_command};
pub use delta::Unpacked;
pub use health::{HttpCheck, Probe};
pub use inspect::{FileInfo, FileKind, FileText, READ_CAP, ReleaseContents, Source};
pub use pm2::{PM2_NEWEST_MAJOR, app_state, parse_version};
pub use preflight::Facts;
pub use releases::{Kind, Listing, ReleaseEntry, RotationPlan, Status, plan_rotation};

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::{Result, bail};

use crate::config::{Deployment, Manager, ManagerKind, release_path};
use crate::deploy::report::{LogKind, Reporter};
use crate::release::is_safe_id;
use crate::ssh::{Output, Session, Stream, sh_quote};
use shipdeck_protocol::message::agent::{AgentManager, AgentService};

/// One deployment folder, and optionally one app's process inside it.
pub struct Server {
    session: Arc<Session>,
    dir: String,
    /// The app this handle runs commands for, when it has one.
    app: Option<String>,
    /// The process name.
    process: String,
    /// The manager, or `None` for a static app.
    manager: Option<Manager>,
    /// The user the process runs as.
    user: String,
    /// The PM2 file, relative to the release.
    pm2_file: String,
    /// What the process gets when it starts.
    env: BTreeMap<String, String>,
    /// The app's environment file, relative to the release.
    env_file: Option<String>,
    /// A static app runs nothing.
    static_site: bool,
}

impl Server {
    /// A handle for the whole deployment folder.
    #[must_use]
    pub fn new(session: Arc<Session>, dir: String) -> Self {
        let user = session.user.clone();
        Self {
            session,
            dir,
            app: None,
            process: String::new(),
            manager: None,
            user,
            pm2_file: String::new(),
            env: BTreeMap::new(),
            env_file: None,
            static_site: false,
        }
    }

    /// A handle for one app of a deploy.
    ///
    /// # Errors
    /// Returns an error when the app name is unsafe.
    pub fn for_app(deployment: &Deployment, app: &str, session: Arc<Session>) -> Result<Self> {
        ensure_app_name(app)?;
        let manager = deployment.manager(app);
        let static_site = deployment.is_static(app);
        let process = if manager.is_some() {
            deployment.process_name(app)
        } else {
            String::new()
        };
        let mut env_file = deployment.env_file(app);
        if let Some(file) = &mut env_file {
            *file = format!(
                "apps/{app}/{}",
                release_path(file).unwrap_or_else(|_| file.clone())
            );
        }
        Ok(Self {
            session,
            dir: deployment.dir().to_string(),
            app: Some(app.to_string()),
            process,
            manager,
            user: deployment.run_user(app),
            pm2_file: deployment.pm2_file(app),
            env: deployment.app_env(app),
            env_file,
            static_site,
        })
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Shares the connection while an operation holds its project lock.
    #[must_use]
    pub fn session_handle(&self) -> Arc<Session> {
        Arc::clone(&self.session)
    }

    #[must_use]
    pub fn dir(&self) -> &str {
        &self.dir
    }

    #[must_use]
    pub fn manager(&self) -> Option<&Manager> {
        self.manager.as_ref()
    }

    /// Which manager it is, without its settings.
    #[must_use]
    pub fn manager_kind(&self) -> Option<ManagerKind> {
        self.manager.as_ref().map(Manager::kind)
    }

    /// As messages name the manager.
    #[must_use]
    pub fn manager_label(&self) -> &'static str {
        match &self.manager {
            Some(manager) => manager.label(),
            None => "static",
        }
    }

    /// The process name, or an empty string for a static app.
    #[must_use]
    pub fn process_name(&self) -> &str {
        &self.process
    }

    /// The app this handle belongs to.
    #[must_use]
    pub fn app(&self) -> Option<&str> {
        self.app.as_deref()
    }

    /// Whether this app serves files and runs nothing.
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.static_site
    }

    fn q_dir(&self) -> String {
        sh_quote(&self.dir)
    }

    fn q_service(&self) -> String {
        sh_quote(&self.process)
    }

    /// Where the bundle is uploaded before it is unpacked.
    #[must_use]
    pub fn upload_path(&self, id: &str) -> String {
        format!("{}/releases/.{id}.tar.zst", self.dir)
    }

    /// The process as the agent's typed requests carry it.
    ///
    /// # Errors
    /// Returns an error for a static app, which has no process.
    pub fn agent_service(&self) -> Result<AgentService> {
        let Some(manager) = self.manager.as_ref() else {
            bail!("a static app has no process");
        };
        let manager = match manager {
            Manager::Systemd(_) => AgentManager::Systemd,
            Manager::Pm2(_) => AgentManager::Pm2,
            Manager::Oxmgr(_) => bail!("oxmgr is not built yet"),
        };
        Ok(AgentService {
            root: self.dir.clone(),
            service: self.process.clone(),
            manager,
            user: self.user.clone(),
            pm2_file: self.pm2_file.clone(),
            env: self.env.clone(),
            env_file: self.env_file.clone(),
        })
    }

    /// The commands that need root here; see [`sudo_commands`].
    #[must_use]
    pub fn sudo_commands(&self) -> Vec<String> {
        match self.manager {
            Some(Manager::Systemd(_)) => sudo_commands(&self.dir, &self.process),
            _ => Vec::new(),
        }
    }

    /// Logged in as root, so nothing needs sudo.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.session.user == "root"
    }

    /// `sudo -n `, or nothing for root.
    fn sudo(&self) -> &'static str {
        if self.is_root() { "" } else { "sudo -n " }
    }

    /// Creates `dir` for an app's first deploy: as the user if it may, else
    /// with sudo (then handed to the user), else it says so.
    ///
    /// # Errors
    /// Returns an error when the folder cannot be created.
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
    ///
    /// # Errors
    /// Returns an error when the folders cannot be created.
    pub async fn prepare(&self, report: &Reporter) -> Result<()> {
        if let Some(agent) = self.session.agent() {
            report.log(
                LogKind::Command,
                format!(
                    "{} › create releases/, backups/ and shared/",
                    self.session.label
                ),
            );
            report.log(LogKind::Info, "creating the release folders");
            agent.prepare_project(&self.dir).await?;
            report.log(LogKind::Info, "checking unfinished release files");
            let inspection = agent.inspect_work(&self.dir).await?;
            if let Some(release) = &inspection.uncertain {
                report.log(
                    LogKind::Warn,
                    format!(
                        "the switch to {release} started but was never confirmed; \
                         check the live release and the service before deploying"
                    ),
                );
            }
            if !inspection.unfinished.is_empty() {
                report.log(
                    LogKind::Warn,
                    format!(
                        "unfinished root files: {}",
                        inspection.unfinished.join(", ")
                    ),
                );
            }
        } else {
            let dir = self.q_dir();
            report.log(LogKind::Info, "creating the release folders");
            self.run(
                "create releases/, backups/ and shared/",
                &format!("cd {dir} && mkdir -p releases backups shared"),
                report,
            )
            .await?;
        }
        Ok(())
    }

    /// Removes every trace of a release that did not make it, its backup
    /// and the files sent to unpack it included.
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe or removal fails.
    pub async fn discard(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        if let Some(agent) = self.session.agent() {
            report.log(
                LogKind::Command,
                format!("{} › remove releases/{id}", self.session.label),
            );
            agent.discard_release(&self.dir, id).await?;
            return Ok(());
        }
        let script = discard_script(&self.dir, id);
        self.run(&format!("remove releases/{id}"), &script, report)
            .await?;
        Ok(())
    }

    /// Points `current` at `releases/<id>` in one rename, so it is never
    /// missing or half-written.
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe or the switch fails.
    pub async fn switch(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        if let Some(agent) = self.session.agent() {
            report.log(
                LogKind::Command,
                format!("{} › point current at {id}", self.session.label),
            );
            agent.switch_release(&self.dir, id).await?;
            return Ok(());
        }
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

    /// Installs the unit `current` carries in `.shipdeck/`, reloads systemd
    /// and enables the service at boot.
    ///
    /// # Errors
    /// Returns an error when the unit cannot be installed or enabled.
    pub async fn install_unit(&self, report: &Reporter) -> Result<()> {
        if self.manager_kind() != Some(ManagerKind::Systemd) {
            return Ok(());
        }
        let (dir, svc, sudo) = (&self.dir, &self.process, self.sudo());
        let script = format!(
            "set -e
{sudo}install -m 0644 {src} {dst}
{sudo}systemctl daemon-reload
{sudo}systemctl enable {svc_q} 2>&1",
            src = sh_quote(&format!("{dir}/current/.shipdeck/{svc}.service")),
            dst = sh_quote(&format!("/etc/systemd/system/{svc}.service")),
            svc_q = self.q_service(),
        );
        self.run(&format!("install {svc}.service"), &script, report)
            .await?;
        Ok(())
    }

    /// Restarts the service on the release `current` points to.
    ///
    /// # Errors
    /// Returns an error when the service cannot be restarted.
    pub async fn restart(&self, report: &Reporter) -> Result<()> {
        match self.manager {
            None => return Ok(()),
            Some(Manager::Pm2(_)) => {
                let script = self.pm2_script("startOrReload");
                self.run(&format!("reload {} in pm2", self.process), &script, report)
                    .await?;
                return Ok(());
            }
            Some(Manager::Oxmgr(_)) => bail!("oxmgr is not built yet"),
            Some(Manager::Systemd(_)) => {}
        }
        let script = format!("{}systemctl restart {}", self.sudo(), self.q_service());
        self.run(&format!("restart {}", self.process), &script, report)
            .await?;
        Ok(())
    }

    /// `pm2 <verb>` on `current`'s PM2 file, for this app only, with the
    /// service's environment loaded; then `pm2 save`.
    fn pm2_script(&self, verb: &str) -> String {
        let current = format!("{}/current", self.dir);
        let file = sh_quote(&format!("{current}/{}", self.pm2_file));
        let env_file = self
            .env_file
            .as_ref()
            .map(|file| format!("{current}/{file}"));
        let env = CommandEnv {
            service: self.env.clone(),
            env_file: None,
            vars: Vec::new(),
        };
        let mut script = env_prelude(&current, &env, env_file);
        let _ = write!(
            script,
            "[ -f {file} ] || {{ echo {missing} >&2; exit 1; }}
pm2 {verb} {file} --only {app} --update-env
pm2 save
",
            missing = sh_quote(&format!(
                "this release has no {}; was it deployed with systemd?",
                self.pm2_file
            )),
            app = sh_quote(&self.process),
        );
        script
    }

    /// Links each shared folder into `releases/<id>`, creating it in
    /// `shared/` the first time.
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe or linking fails.
    pub async fn link_shared(&self, id: &str, shared: &[String], report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        if shared.is_empty() {
            return Ok(());
        }
        if let Some(agent) = self.session.agent() {
            report.log(
                LogKind::Command,
                format!(
                    "{} › link shared folders into the release",
                    self.session.label
                ),
            );
            let paths: Vec<String> = shared
                .iter()
                .map(|entry| release_path(entry).unwrap_or_else(|_| entry.clone()))
                .collect();
            agent.link_shared(&self.dir, id, &paths).await?;
            return Ok(());
        }
        let mut script = format!("set -e\ncd {}\n", self.q_dir());
        for entry in shared {
            let entry = release_path(entry).unwrap_or_else(|_| entry.clone());
            let link = sh_quote(&format!("releases/{id}/{entry}"));
            let _ = write!(
                script,
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
            );
        }
        self.run("link shared folders into the release", &script, report)
            .await?;
        Ok(())
    }

    /// Runs one command from the deploy file at `place`, streaming its
    /// output. It sees `process.env`, then `env_file` (as systemd reads it:
    /// no shell expansion), then `vars`.
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe or the command fails.
    pub async fn command(
        &self,
        place: &Place,
        command: &str,
        env: &CommandEnv,
        report: &Reporter,
    ) -> Result<()> {
        let (dir, shown, env_file) = self.place(place, env)?;
        let mut script = env_prelude(&dir, env, env_file);
        script.push_str(&crate::remote::lease::prelude(
            &self.dir,
            &crate::remote::lease::new_token(),
        ));
        // The command is shell by design: it comes from the deploy file,
        // written by whoever deploys, and runs as the deploy user.
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

    /// The folder, its label, and the environment file for a place.
    fn place(&self, place: &Place, env: &CommandEnv) -> Result<(String, String, Option<String>)> {
        let (dir, shown, release) = match place {
            Place::Root => (
                self.dir.clone(),
                "in dir".to_string(),
                "current".to_string(),
            ),
            Place::Release(id) => {
                ensure_id(id)?;
                (
                    format!("{}/releases/{id}", self.dir),
                    format!("in releases/{id}"),
                    format!("releases/{id}"),
                )
            }
            Place::Current => (
                format!("{}/current", self.dir),
                "in current".to_string(),
                "current".to_string(),
            ),
            Place::AppRelease(id, app) => {
                ensure_id(id)?;
                ensure_app_name(app)?;
                (
                    format!("{}/releases/{id}/apps/{app}", self.dir),
                    format!("in releases/{id}/apps/{app}"),
                    format!("releases/{id}"),
                )
            }
            Place::AppCurrent(app) => {
                ensure_app_name(app)?;
                (
                    format!("{}/current/apps/{app}", self.dir),
                    format!("in current/apps/{app}"),
                    "current".to_string(),
                )
            }
        };
        let env_file = env
            .env_file
            .as_ref()
            .map(|file| format!("{}/{release}/{file}", self.dir));
        Ok((dir, shown, env_file))
    }

    /// The service's last 30 log lines: its journal, or PM2's logs.
    ///
    /// # Errors
    /// Returns an error when the log command cannot run.
    pub async fn journal(&self) -> Result<Vec<String>> {
        let script = match self.manager {
            Some(Manager::Systemd(_)) => format!(
                "{}journalctl -u {} -n 30 --no-pager 2>&1",
                self.sudo(),
                self.q_service()
            ),
            Some(Manager::Pm2(_)) => format!(
                "pm2 logs {} --lines 30 --nostream --raw 2>&1",
                sh_quote(&self.process)
            ),
            Some(Manager::Oxmgr(_)) | None => {
                bail!("a static site has no process logs; nginx keeps its own in /var/log/nginx")
            }
        };
        let out = self.session.exec(&script).await?;
        Ok(out.stdout.lines().map(str::to_string).collect())
    }
}

/// `set -e`, `cd dir`, then `env` exported: `process.env`, then `env_file`
/// (as systemd reads it), then `vars`.
fn env_prelude(dir: &str, env: &CommandEnv, env_file: Option<String>) -> String {
    let mut script = format!("set -e\ncd {}\n", sh_quote(dir));
    for (key, value) in &env.service {
        let _ = writeln!(script, "export {key}={}", sh_quote(value));
    }
    if let Some(file) = env_file {
        script.push_str(ENV_FILE_READER);
        let _ = writeln!(script, "shipdeck_env {}", sh_quote(&file));
    }
    for (key, value) in &env.vars {
        let _ = writeln!(script, "export {key}={}", sh_quote(value));
    }
    script
}

/// Where a command from the deploy file runs on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `dir`, for preflight and for rollback and failed commands.
    Root,
    /// `releases/<id>`, while the old release is still live.
    Release(String),
    /// `current`, for health and done.
    Current,
    /// An app's folder in a new release.
    AppRelease(String, String),
    /// An app's folder in the live release.
    AppCurrent(String),
}

/// Checks an app name before using it in a server path.
fn ensure_app_name(name: &str) -> Result<()> {
    if name.starts_with('.') || !crate::config::is_unit_name(name) {
        bail!("invalid app name {name:?}");
    }
    Ok(())
}

/// What a server command sees besides the deploy user's environment.
#[derive(Debug, Clone, Default)]
pub struct CommandEnv {
    /// `process.env`.
    pub service: BTreeMap<String, String>,
    /// The environment file, relative to the release.
    pub env_file: Option<String>,
    /// `SHIPDECK_*`.
    pub vars: Vec<(String, String)>,
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
#[must_use]
pub fn sudo_commands(dir: &str, service: &str) -> Vec<String> {
    let svc = service;
    vec![
        "systemctl daemon-reload".into(),
        format!("systemctl enable {svc}"),
        format!("systemctl restart {svc}"),
        format!(
            "install -m 0644 {dir}/current/.shipdeck/{svc}.service /etc/systemd/system/{svc}.service"
        ),
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

pub(crate) fn ensure_id(id: &str) -> Result<()> {
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
