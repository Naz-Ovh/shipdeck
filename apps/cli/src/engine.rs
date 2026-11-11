//! `shipdeck engine`: the process an app starts, and the pipe it holds.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};

use shipdeck_protocol::connection::Connection;
use shipdeck_protocol::message::common::Login;
use shipdeck_protocol::message::request::{Initialize, WorkspaceOpen, WorkspaceOpenResult};

/// The environment variable that carries the shell a local command runs in.
#[cfg(windows)]
const GITSHELL_ENV: &str = "SHIPDECK_GITSHELL_PATH";

/// Runs as the engine: the protocol on stdin and stdout, for an app to draw.
///
/// # Errors
/// Returns an error when it is typed at a terminal, which carries no protocol.
pub async fn serve() -> Result<()> {
    // Typed at a terminal it would read the keyboard and answer nowhere.
    if std::io::stdin().is_terminal() {
        bail!(
            "`shipdeck engine` talks to an app over a pipe, one message per line; \
             see `shipdeck man details cli`"
        );
    }
    #[cfg(windows)]
    if let Some(path) = std::env::var_os(GITSHELL_ENV) {
        shipdeck_engine::build::set_gitshell_path(std::path::PathBuf::from(path));
    }
    shipdeck_engine::rpc::run(tokio::io::stdin(), tokio::io::stdout()).await;
    Ok(())
}

/// `shipdeck engine`, started for this run: one folder, one pipe, then gone.
pub struct Engine {
    child: Child,
    connection: Connection,
}

impl Engine {
    /// Starts the engine, and agrees on the protocol's version.
    ///
    /// # Errors
    /// Fails when the child cannot be started, or does not answer.
    pub async fn start(gitshell: Option<&Path>) -> Result<Self> {
        let mut child = Command::from(line(gitshell)?)
            .kill_on_drop(true)
            .spawn()
            .context("starting `shipdeck engine`")?;
        let read = child.stdout.take().context("the engine's stdout")?;
        let write = child.stdin.take().context("the engine's stdin")?;
        let connection = Connection::start(read, write);
        let hello = connection
            .initialize(&Initialize {
                protocol: shipdeck_protocol::VERSION,
                client: name(),
            })
            .await
            .context("the engine did not answer initialize")?;
        if hello.protocol != shipdeck_protocol::VERSION {
            bail!(
                "the engine speaks protocol {}, and this program speaks {}",
                hello.protocol,
                shipdeck_protocol::VERSION
            );
        }
        Ok(Self { child, connection })
    }

    /// Opens the folder of projects; every later message names one of them.
    ///
    /// # Errors
    /// Fails when the engine does not answer, or the folder is not one.
    pub async fn open(
        &self,
        root: &Path,
        login: &Login,
        no_shell_env: bool,
        config: Option<&Path>,
        deploy_files: &[PathBuf],
    ) -> Result<WorkspaceOpenResult> {
        self.connection
            .workspace_open(&WorkspaceOpen {
                root: root.display().to_string(),
                config: config.map(|file| file.display().to_string()),
                deploy_files: deploy_files
                    .iter()
                    .map(|file| file.display().to_string())
                    .collect(),
                login: login.clone(),
                no_shell_env,
            })
            .await
            .context("opening the project")
    }

    /// The pipe, for the command that drives it, and the process to wait for.
    pub fn into_parts(self) -> (Connection, Child) {
        (self.connection, self.child)
    }
}

/// This program, run as the engine, with its three streams wired up.
#[cfg(windows)]
fn line(gitshell: Option<&Path>) -> Result<std::process::Command> {
    let program = std::env::current_exe().context("finding this program")?;
    let mut command = std::process::Command::new(program);
    command
        .arg("engine")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Its warnings are the app's to print; the protocol is on stdout.
        .stderr(Stdio::inherit());
    own_group(&mut command);
    if let Some(path) = gitshell {
        command.env(GITSHELL_ENV, path);
    }
    Ok(command)
}

/// This program, run as the engine. Windows' `gitshell` flag has no use here.
#[cfg(not(windows))]
fn line(_: Option<&Path>) -> Result<std::process::Command> {
    let program = std::env::current_exe().context("finding this program")?;
    let mut command = std::process::Command::new(program);
    command
        .arg("engine")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    own_group(&mut command);
    Ok(command)
}

/// The engine must not see the terminal's Ctrl-C: the app decides, and the
/// pipe closes when the app goes.
#[cfg(unix)]
fn own_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn own_group(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP: this console's Ctrl-C is not the engine's.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

/// Who is talking, as `initialize` says: `shipdeck 0.6.1`.
fn name() -> String {
    format!("shipdeck {}", env!("CARGO_PKG_VERSION"))
}
