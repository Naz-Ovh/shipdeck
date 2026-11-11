//! Running a step's commands: the runner it names, the folder it runs in,
//! and the placeholders and environment variables it sees.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use super::report::{LogKind, Reporter};
use super::steps::{self, Action, Commands as Scope, Step};
use crate::build;
use crate::config::placeholders::{self, Values};
use crate::config::{Command, CommandList, Deployment, Runner, release_path};
use crate::remote::{CommandEnv, Place, Server};
use crate::ssh::{Session, sh_quote};

/// One command scope: the deployment, the row it reports to, and what the
/// placeholders see.
pub struct Exec<'a> {
    pub deployment: &'a Deployment,
    pub report: &'a Reporter,
    pub session: Option<Arc<Session>>,
    pub release: Option<String>,
    pub version: Option<String>,
    pub previous: Option<String>,
    pub failed_step: Option<Step>,
    /// The operation this command belongs to.
    pub action: Action,
    /// The step row progress goes to.
    pub row: usize,
}

impl Exec<'_> {
    /// Runs a step's commands: the server's first, release then apps, then
    /// the local ones.
    ///
    /// # Errors
    /// Returns the first command that fails.
    pub async fn step(&self, step: Step, error: Option<&str>) -> Result<()> {
        for scope in steps::scopes(self.deployment, step) {
            Box::pin(self.server(step, scope, error)).await?;
        }
        for scope in steps::scopes(self.deployment, step) {
            Box::pin(self.local(step, scope, error)).await?;
        }
        Ok(())
    }

    /// Runs one scope's server commands.
    ///
    /// # Errors
    /// Returns the first command that fails.
    pub async fn server(&self, step: Step, scope: Scope<'_>, error: Option<&str>) -> Result<()> {
        if scope.server.is_empty() {
            return Ok(());
        }
        let server = self.server_for(scope.app)?;
        let place = self.place(step, scope.app)?;
        let env = self.env(step, scope.app, error)?;
        let total = scope.server.len() as u64;
        for (done, command) in scope.server.iter().enumerate() {
            let done = done as u64;
            let line = self.line(command, scope.app)?;
            self.progress(step, done, total, "server", &line);
            server.command(&place, &line, &env, self.report).await?;
            self.progress(step, done + 1, total, "server", &line);
        }
        Ok(())
    }

    /// Runs one scope's local commands.
    ///
    /// # Errors
    /// Returns the first command that fails.
    pub async fn local(&self, step: Step, scope: Scope<'_>, error: Option<&str>) -> Result<()> {
        if scope.local.is_empty() {
            return Ok(());
        }
        let dir = self.local_dir(scope.app);
        let env = self.env(step, scope.app, error)?;
        let total = scope.local.len() as u64;
        for (done, command) in scope.local.iter().enumerate() {
            let done = done as u64;
            let line = self.line(command, scope.app)?;
            self.progress(step, done, total, "local", &line);
            Box::pin(build::run_command(
                command,
                &self.code(command, scope.app)?,
                &dir,
                &env.vars,
                self.report,
                self.row,
            ))
            .await?;
            self.progress(step, done + 1, total, "local", &line);
        }
        Ok(())
    }

    /// A command as the shell line the server runs, with its placeholders
    /// replaced first, so a value with a quote is still quoted as a whole.
    pub fn line(&self, command: &Command, app: Option<&str>) -> Result<String> {
        let code = self.code(command, app)?;
        Ok(match command {
            Command::Sh(_) => code,
            Command::Runner { runner, .. } => runner_line(*runner, &code),
        })
    }

    /// The code a local runner gets, with its placeholders replaced.
    pub fn code(&self, command: &Command, app: Option<&str>) -> Result<String> {
        let code = match command {
            Command::Sh(code) | Command::Runner { code, .. } => code,
        };
        self.deployment
            .expand(code, self.release.as_deref(), self.version.as_deref(), app)
    }

    fn progress(&self, step: Step, done: u64, total: u64, side: &str, text: &str) {
        self.report.progress(
            self.row,
            done,
            total,
            super::report::Unit::Items,
            format!("{} {side}: {}", step.name(), text.trim()),
        );
    }

    /// A server handle for one app, or the deployment folder.
    ///
    /// # Errors
    /// Returns an error when the app name is unsafe.
    pub fn server_for(&self, app: Option<&str>) -> Result<Server> {
        let session = self
            .session
            .clone()
            .ok_or_else(|| anyhow::anyhow!("there is no server connection"))?;
        match app {
            Some(name) => Server::for_app(self.deployment, name, session),
            None => Ok(Server::new(session, self.deployment.dir().to_string())),
        }
    }

    /// The folder a local command runs in.
    #[must_use]
    pub fn local_dir(&self, app: Option<&str>) -> PathBuf {
        match app {
            Some(name) => self.deployment.source(name),
            None => self.deployment.project.clone(),
        }
    }

    /// Where a step's server commands run.
    ///
    /// # Errors
    /// Returns an error when a release folder is needed but not named.
    pub fn place(&self, step: Step, app: Option<&str>) -> Result<Place> {
        let release = || {
            self.release
                .clone()
                .ok_or_else(|| anyhow::anyhow!("the release is not named"))
        };
        Ok(match (step, app) {
            (Step::Preflight | Step::Build | Step::Upload | Step::Rollback | Step::Failed, _) => {
                Place::Root
            }
            (Step::Setup | Step::Backup | Step::Switch, Some(name)) => {
                Place::AppRelease(release()?, name.to_string())
            }
            (Step::Setup | Step::Backup | Step::Switch, None) => Place::Release(release()?),
            (Step::Restart | Step::Nginx | Step::Health | Step::Done, Some(name)) => {
                Place::AppCurrent(name.to_string())
            }
            (Step::Restart | Step::Nginx | Step::Health | Step::Done, None) => Place::Current,
        })
    }

    /// The placeholders' values here.
    #[must_use]
    pub fn values<'a>(&'a self, app: Option<&'a str>) -> Values<'a> {
        self.deployment
            .values(self.release.as_deref(), self.version.as_deref(), app)
    }

    /// What a server command sees: the process environment, its env file,
    /// and the variables.
    ///
    /// # Errors
    /// Returns a message for a placeholder with no value here.
    pub fn env(&self, step: Step, app: Option<&str>, error: Option<&str>) -> Result<CommandEnv> {
        let mut vars = placeholders::env(&self.values(app));
        vars.push(("SHIPDECK_ACTION".into(), self.action.key().into()));
        let failed = if matches!(step, Step::Failed | Step::Rollback) {
            self.failed_step.unwrap_or(step)
        } else {
            step
        };
        vars.push(("SHIPDECK_STEP".into(), failed.key().into()));
        if let Some(previous) = &self.previous {
            vars.push(("SHIPDECK_PREVIOUS".into(), previous.clone()));
        }
        if let Some(error) = error {
            vars.push(("SHIPDECK_ERROR".into(), error.to_string()));
        }
        let fill = |text: &str, app: Option<&str>| {
            self.deployment
                .expand(text, self.release.as_deref(), self.version.as_deref(), app)
        };
        let service = match app {
            None => BTreeMap::new(),
            Some(name) => self
                .deployment
                .app_env(name)
                .into_iter()
                .map(|(key, value)| Ok((key, fill(&value, Some(name))?)))
                .collect::<Result<BTreeMap<_, _>>>()?,
        };
        let env_file = app
            .and_then(|name| self.deployment.env_file(name))
            .map(|name_in_file| {
                let expanded = fill(&name_in_file, app).unwrap_or(name_in_file);
                format!(
                    "apps/{}/{}",
                    app.unwrap_or_default(),
                    release_path(&expanded).unwrap_or(expanded)
                )
            });
        Ok(CommandEnv {
            service,
            env_file,
            vars,
        })
    }
}

/// A runner's shell line for `code`.
#[must_use]
pub fn runner_line(runner: Runner, code: &str) -> String {
    format!("{} {} {}", runner.key(), runner.flag(), sh_quote(code))
}

/// The runners used by a command list, with the step and app that use them.
#[must_use]
pub fn runners(list: &CommandList) -> Vec<Runner> {
    list.iter()
        .map(|command| match command {
            Command::Sh(_) => Runner::Sh,
            Command::Runner { runner, .. } => *runner,
        })
        .collect()
}

/// Says why a missing local runner stops the deploy.
///
/// # Errors
/// Returns the message naming the runner, step and app.
pub fn missing_local(runner: Runner, step: Step, app: Option<&str>) -> anyhow::Error {
    let app = app.map_or_else(String::new, |name| format!(" in app {name}"));
    anyhow::anyhow!(
        "the {} runner used by `{}`{app} is not on your PC",
        runner.key(),
        step.key()
    )
}

/// Reports a warning that server commands were skipped.
pub fn skipped(report: &Reporter, reason: &str) {
    report.log(
        LogKind::Warn,
        format!("server commands were skipped: {reason}"),
    );
}
