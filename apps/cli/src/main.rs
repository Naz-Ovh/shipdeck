//! shipdeck: build on this machine, deploy over SSH, roll back any release.
//! The flags pick the terminal UI (`shipdeck-tui`) or a console command
//! (`console`); both drive `shipdeck-engine` as a child process.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use tokio::sync::mpsc::unbounded_channel;

use shipdeck_engine::config::{self, Discovery, Manager};
use shipdeck_engine::init::Preset;
use shipdeck_engine::ssh::Login;
use shipdeck_protocol::message::{DeployableInfo, Problem};

use console::{Choices, Console};
use shipdeck_protocol::message::common::{Login as ProtocolLogin, OnFailure};
use shipdeck_protocol::message::request::SetupAccess;

mod console;
mod engine;
mod init;
mod man;

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The password, for runs where `--password` would land in a log.
const PASSWORD_ENV: &str = "SHIPDECK_PASSWORD";
/// A private key's text, such as a CI secret, kept out of `ps` and logs.
const KEY_ENV: &str = "SHIPDECK_SSH_KEY";

#[derive(Parser)]
#[command(
    name = "shipdeck",
    version,
    about = "Build locally, deploy over SSH as a systemd service or PM2 app, roll back any release.",
    after_help = "Each folder under DEPLOY_ROOT with a deploy.yaml is one deployable.\n\
                  Login, first found: --key, --key-text, --password, SHIPDECK_SSH_KEY,\n\
                  SHIPDECK_PASSWORD; else the host's ~/.ssh/config key, then the TUI asks\n\
                  for the password. SHIPDECK_KEY_PASSPHRASE unlocks an encrypted key.\n\
                  deploy, rollback and releases run without the TUI and ask nothing.\n\
                  init creates a deployable folder; it asks for what its flags leave out.\n\
                  man is the full manual: `shipdeck man index`, `man configs`, `man details <KEY>...`.",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    /// Folder whose subfolders each hold a deploy.yaml
    #[arg(value_name = "DEPLOY_ROOT", required_unless_present = "schema")]
    root: Option<PathBuf>,

    #[command(flatten)]
    login: LoginArgs,

    /// Print the JSON Schema of deploy.yaml, for editor completion
    #[arg(long)]
    schema: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args, Clone)]
struct LoginArgs {
    /// Log in with this private key file
    #[arg(long, value_name = "PATH", conflicts_with_all = ["password", "key_text"])]
    key: Option<PathBuf>,

    /// Log in with this private key's text (it shows in `ps`; `SHIPDECK_SSH_KEY` does not)
    #[arg(long, value_name = "TEXT", conflicts_with = "password")]
    key_text: Option<String>,

    /// Log in with this password (it lands in your shell history; `SHIPDECK_PASSWORD` does not)
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,

    /// Run server commands without the login shell's startup files (~/.bashrc, ~/.zshrc, ...), on every server
    #[arg(long)]
    no_shell_env: bool,

    /// Run local commands with this sh.exe instead of the one Git for Windows has
    #[cfg(windows)]
    #[arg(long, value_name = "PATH", env = "SHIPDECK_GITSHELL_PATH")]
    gitshell_path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Create a deployable folder with a deploy.yaml from a preset; asks for what the flags leave out
    Init(InitArgs),
    /// Run as the engine: it answers the app on stdin and stdout, one message per line
    Engine,
    #[command(flatten)]
    Server(ServerCommand),
    /// The manual, for people and coding agents: `man index` (topics), `man configs` (deploy.yaml), `man details <KEY>...`
    Man(man::ManArgs),
}

/// The commands that log in to a server.
#[derive(Subcommand)]
enum ServerCommand {
    /// Build and deploy one or more deployables, one after another, without the TUI
    Deploy {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,

        /// Deployables to deploy, by folder name or `name`; the first failure stops the rest
        #[arg(value_name = "NAME", required = true)]
        names: Vec<String>,

        /// What to do when the new release fails after the switch; overrides `pipeline.failed.rollback`
        #[arg(long, value_enum, default_value_t = OnFailureArg::Rollback)]
        on_failure: OnFailureArg,

        /// Deploy even when nothing changed since the live release; without it, such a deploy stops with exit code 6
        #[arg(long)]
        allow_unchanged: bool,

        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Switch a deployable to a release already on the server, without the TUI
    Rollback {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,

        /// The deployable, by folder name or `name`
        #[arg(value_name = "NAME")]
        name: String,

        /// The release id to make live, as `shipdeck releases` lists it
        #[arg(long, value_name = "RELEASE")]
        to: String,

        #[command(flatten)]
        ssh: SshArgs,
    },
    /// List the releases on the server, newest first
    Releases {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,

        /// The deployable, by folder name or `name`
        #[arg(value_name = "NAME")]
        name: String,

        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Show Linux, CPU, memory, disk, uptime, and installed tools on a server
    ServerInfo {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,
        /// The deployable, by folder name or `name`
        #[arg(value_name = "NAME")]
        name: String,
        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Show what an interrupted operation left, and clean it up with --cleanup
    Work {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,
        /// The deployable, by folder name or `name`
        #[arg(value_name = "NAME")]
        name: String,
        /// Remove the unfinished files this command lists; this flag is the approval
        #[arg(long)]
        cleanup: bool,
        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Print the events saved on the server, oldest first
    Events {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,
        /// The deployable, by folder name or `name`
        #[arg(value_name = "NAME")]
        name: String,
        /// Only events with a larger sequence number
        #[arg(long, default_value_t = 0)]
        after: i64,
        /// At most this many events
        #[arg(long, default_value_t = 200)]
        limit: u32,
        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Prepare a server once: packages, the project folder, and sudo access
    Setup {
        /// Folder whose subfolders each hold a deploy.yaml
        #[arg(value_name = "DEPLOY_ROOT")]
        root: PathBuf,
        /// The deployable, by folder name or `name`
        #[arg(value_name = "NAME")]
        name: String,
        /// Password-free sudo for every command, or only the ones shipdeck runs
        #[arg(long, value_enum)]
        access: Option<AccessArg>,
        /// The sudo password for this run; `SHIPDECK_SUDO_PASSWORD` keeps it out of history
        #[arg(long, value_name = "PASSWORD")]
        sudo_password: Option<String>,
        #[command(flatten)]
        ssh: SshArgs,
    },
}

#[derive(Args)]
struct InitArgs {
    /// Folder whose subfolders each hold a deploy.yaml
    #[arg(value_name = "DEPLOY_ROOT")]
    root: PathBuf,

    /// The new folder's name, also the service's name, such as my-api
    #[arg(value_name = "NAME")]
    name: Option<String>,

    /// What the app is written in, which sets how it is built and run
    #[arg(long, value_parser = preset_parser())]
    preset: Option<Preset>,

    /// What keeps the app running on the server [default: systemd]
    #[arg(long, value_enum)]
    manager: Option<ManagerArg>,

    /// The server: an address, or an alias from ~/.ssh/config
    #[arg(long)]
    host: Option<String>,

    /// The user to log in and deploy as [default: the alias's User]
    #[arg(long)]
    user: Option<String>,

    /// Your source folder [default: the current folder]
    #[arg(long, value_name = "PATH")]
    project: Option<PathBuf>,
}

/// `--access`, as the flag parser knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AccessArg {
    /// Password-free sudo for every command
    Full,
    /// Password-free sudo for exactly the commands shipdeck runs
    Commands,
}

impl From<AccessArg> for SetupAccess {
    fn from(arg: AccessArg) -> Self {
        match arg {
            AccessArg::Full => SetupAccess::Full,
            AccessArg::Commands => SetupAccess::Commands,
        }
    }
}

/// `--on-failure`, as the flag parser knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OnFailureArg {
    /// Put the release that was live before back at once
    Rollback,
    /// Keep the failed release live, to look at it
    Keep,
}

impl From<OnFailureArg> for OnFailure {
    fn from(arg: OnFailureArg) -> Self {
        match arg {
            OnFailureArg::Rollback => OnFailure::Rollback,
            OnFailureArg::Keep => OnFailure::Keep,
        }
    }
}

/// `--preset`: the engine's names, so the list is never kept twice.
fn preset_parser() -> impl TypedValueParser<Value = Preset> {
    PossibleValuesParser::new(Preset::ALL.map(Preset::name))
        .map(|name| Preset::from_name(&name).expect("the parser only passes listed names"))
}

/// `--manager`, as the flag parser knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ManagerArg {
    /// A systemd unit, installed and restarted with sudo
    Systemd,
    /// PM2, run as the SSH user without sudo
    Pm2,
}

impl From<ManagerArg> for Manager {
    fn from(arg: ManagerArg) -> Self {
        match arg {
            ManagerArg::Systemd => Manager::Systemd,
            ManagerArg::Pm2 => Manager::Pm2,
        }
    }
}

#[derive(Args)]
struct SshArgs {
    #[command(flatten)]
    login: LoginArgs,

    /// Trust and record the host key of a server not yet in your `known_hosts` file; a changed key is still refused
    #[arg(long)]
    trust_new_host: bool,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let cli = Cli::from_arg_matches(&flags().get_matches()).unwrap_or_else(|err| err.exit());
    if cli.schema {
        print!("{}", config::schema::text());
        return Ok(ExitCode::SUCCESS);
    }
    let command = match cli.command {
        Some(Command::Init(args)) => return run_init(args),
        Some(Command::Man(args)) => return Ok(man::run(args)),
        Some(Command::Engine) => {
            engine::serve().await?;
            // The stdin reader is parked in a blocking read that a runtime
            // shutdown would wait for; everything written is out by now.
            std::process::exit(0);
        }
        Some(Command::Server(command)) => command,
        None => {
            let root = cli
                .root
                .expect("clap requires DEPLOY_ROOT without --schema or a command");
            return run_tui(root, cli.login).await;
        }
    };

    let (root, ssh) = match &command {
        ServerCommand::Deploy { root, ssh, .. }
        | ServerCommand::Rollback { root, ssh, .. }
        | ServerCommand::Releases { root, ssh, .. }
        | ServerCommand::ServerInfo { root, ssh, .. }
        | ServerCommand::Work { root, ssh, .. }
        | ServerCommand::Events { root, ssh, .. }
        | ServerCommand::Setup { root, ssh, .. } => (root.as_path(), ssh),
    };
    let shell_env = !ssh.login.no_shell_env;
    let gitshell = gitshell(&ssh.login);
    let login = login(ssh.login.clone())?;
    // The engine reports the same problems; checking here keeps the caret
    // block, the colours and the exact output scripts have seen until now.
    let Some(_) = discover(root)? else {
        return Ok(ExitCode::FAILURE);
    };
    let engine = engine::Engine::start(gitshell.as_deref()).await?;
    let workspace = engine.open(root, &wire(&login), !shell_env).await?;
    if !workspace.problems.is_empty() {
        return Ok(problems(&workspace.problems));
    }
    let setup = match &command {
        ServerCommand::Setup {
            access,
            sudo_password,
            ..
        } => Some((
            setup_access(*access)?,
            setup_password(sudo_password.clone())?,
        )),
        _ => None,
    };
    let (on_failure, allow_unchanged) = match &command {
        ServerCommand::Deploy {
            on_failure,
            allow_unchanged,
            ..
        } => (OnFailure::from(*on_failure), *allow_unchanged),
        _ => (OnFailure::Rollback, false),
    };
    let (connection, mut child) = engine.into_parts();
    let mut console = Console::new(
        connection,
        Choices {
            trust_new_host: ssh.trust_new_host,
            allow_unchanged,
            on_failure,
        },
    );
    let code = run_command(&mut console, command, &workspace.deployables, setup).await;
    console.close().await;
    let _ = child.wait().await;
    Ok(code)
}

/// Runs the one command that was chosen.
async fn run_command(
    console: &mut Console,
    command: ServerCommand,
    projects: &[DeployableInfo],
    setup: Option<(SetupAccess, Option<String>)>,
) -> ExitCode {
    match command {
        ServerCommand::Deploy { names, .. } => console.deploy(projects, &names).await,
        ServerCommand::Rollback { name, to, .. } => console.rollback(projects, &name, to).await,
        ServerCommand::Releases { name, .. } => console.releases(projects, &name).await,
        ServerCommand::ServerInfo { name, .. } => console.server_info(projects, &name).await,
        ServerCommand::Work { name, cleanup, .. } => console.work(projects, &name, cleanup).await,
        ServerCommand::Events {
            name, after, limit, ..
        } => console.events(projects, &name, after, limit).await,
        ServerCommand::Setup { name, .. } => {
            let Some((access, password)) = setup else {
                unreachable!("setup answers were read above");
            };
            console.setup(projects, &name, access, password).await
        }
    }
}

/// `--access`, or the question when there is someone to ask.
fn setup_access(given: Option<AccessArg>) -> Result<SetupAccess> {
    if let Some(given) = given {
        return Ok(given.into());
    }
    if !init::can_ask() {
        return Ok(SetupAccess::Commands);
    }
    let chosen = inquire::Select::new(
        "Sudo access:",
        vec![
            "Commands only (password-free sudo for what shipdeck runs)",
            "Full sudo (password-free sudo for every command)",
        ],
    )
    .prompt()
    .map_err(|err| anyhow::anyhow!("no access chosen: {err}"))?;
    Ok(if chosen.starts_with("Full") {
        SetupAccess::Full
    } else {
        SetupAccess::Commands
    })
}

/// `--sudo-password`, `SHIPDECK_SUDO_PASSWORD`, or the hidden question.
fn setup_password(given: Option<String>) -> Result<Option<String>> {
    if given.is_some() {
        return Ok(given);
    }
    if let Some(password) = std::env::var("SHIPDECK_SUDO_PASSWORD")
        .ok()
        .filter(|password| !password.is_empty())
    {
        return Ok(Some(password));
    }
    if !init::can_ask() {
        return Ok(None);
    }
    let password = inquire::Password::new("Sudo password (blank when sudo needs none):")
        .without_confirmation()
        .prompt()
        .map_err(|err| anyhow::anyhow!("no sudo password entered: {err}"))?;
    Ok((!password.is_empty()).then_some(password))
}

/// The login as the protocol carries it: the same four ways in.
fn wire(login: &Login) -> ProtocolLogin {
    match login {
        Login::Key(path) => ProtocolLogin::Key {
            path: path.display().to_string(),
        },
        Login::KeyText(text) => ProtocolLogin::KeyText { text: text.clone() },
        Login::Password(password) => ProtocolLogin::Password {
            password: password.clone(),
        },
        Login::Auto => ProtocolLogin::Auto,
    }
}

/// The shell the engine runs local commands in, when the flag names one.
fn gitshell(args: &LoginArgs) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        args.gitshell_path.clone()
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        None
    }
}

/// What the engine found in a file this program let through: say where, and
/// stop. Every project in the folder is refused, as the check above does.
fn problems(found: &[Problem]) -> ExitCode {
    for problem in found {
        eprintln!(
            "{}:{}:{}: {}",
            problem.file, problem.line, problem.column, problem.message
        );
    }
    ExitCode::FAILURE
}

/// The flags. With no terminal to ask in, `init` needs on the command line
/// what it would otherwise ask for.
fn flags() -> clap::Command {
    let can_ask = init::can_ask();
    Cli::command().mut_subcommand("init", |command| {
        ["name", "preset", "host"]
            .into_iter()
            .fold(command, |command, id| {
                command.mut_arg(id, |arg| arg.required(!can_ask))
            })
    })
}

/// `init` logs in nowhere: it only writes files.
fn run_init(args: InitArgs) -> Result<ExitCode> {
    let given = init::Given {
        root: args.root,
        name: args.name,
        preset: args.preset,
        manager: args.manager.map(Into::into),
        host: args.host,
        user: args.user,
        project: args.project,
    };
    init::run(given)
}

async fn run_tui(root: PathBuf, args: LoginArgs) -> Result<ExitCode> {
    let login = login(args.clone())?;
    // The CLI reads every file for its own error report; the engine answers
    // with the projects the TUI then follows.
    if discover(&root)?.is_none() {
        return Ok(ExitCode::FAILURE);
    }
    let engine = engine::Engine::start(gitshell(&args).as_deref()).await?;
    let workspace = engine
        .open(&root, &wire(&login), !args.no_shell_env)
        .await?;
    let mut deployables = Vec::with_capacity(workspace.deployables.len());
    for info in &workspace.deployables {
        let folder = PathBuf::from(&info.folder);
        let Ok(found) = shipdeck_engine::config::Deployable::load(&folder) else {
            bail!(
                "{}: its deploy.yaml cannot be read any more",
                folder.display()
            );
        };
        deployables.push(found);
    }
    let (connection, mut child) = engine.into_parts();
    let handle = connection.handle();
    let (questions_tx, questions) = unbounded_channel();
    let (ops_tx, ops) = unbounded_channel();
    let (fetched_tx, fetched) = unbounded_channel();
    shipdeck_tui::pump(
        connection,
        shipdeck_tui::Inboxes {
            ops: ops_tx,
            questions: questions_tx,
            fetched: fetched_tx.clone(),
        },
    );
    let app = shipdeck_tui::App::new(
        root,
        Discovery {
            deployables,
            monorepos: Vec::new(),
            problems: Vec::new(),
        },
        login,
        handle,
        fetched_tx,
    );
    shipdeck_tui::run(
        app,
        shipdeck_tui::Inbox {
            questions,
            ops,
            fetched,
        },
    )
    .await?;
    let _ = child.wait().await;
    Ok(ExitCode::SUCCESS)
}

fn login(args: LoginArgs) -> Result<Login> {
    if let Some(key) = args.key {
        let key = expand_home(key);
        if !key.is_file() {
            bail!("the key file {} does not exist", key.display());
        }
        return Ok(Login::Key(key));
    }
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    if let Some(text) = args.key_text {
        return Ok(Login::KeyText(text));
    }
    if let Some(password) = args.password {
        return Ok(Login::Password(password));
    }
    if let Some(text) = env(KEY_ENV) {
        return Ok(Login::KeyText(text));
    }
    Ok(match env(PASSWORD_ENV) {
        Some(password) => Login::Password(password),
        None => Login::Auto,
    })
}

/// Every deployable under `root`, or `None` once every problem in every
/// file is printed: nothing runs from a config shipdeck cannot fully read.
fn discover(root: &Path) -> Result<Option<Discovery>> {
    if !root.is_dir() {
        bail!("{} is not a folder", root.display());
    }
    let discovery = config::discover(root)
        .with_context(|| format!("looking for deployables in {}", root.display()))?;
    if discovery.problems.is_empty() {
        return Ok(Some(discovery));
    }
    let base = std::env::current_dir().unwrap_or_default();
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    eprint!(
        "{}",
        config::render(&discovery.problems, &base, color, "shipdeck did not start")
    );
    Ok(None)
}

/// `~/.ssh/id_ed25519` → `/home/me/.ssh/id_ed25519`; zsh expands `~` only
/// at the start of a word, not after `--key=`.
fn expand_home(path: PathBuf) -> PathBuf {
    match (path.strip_prefix("~"), std::env::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path,
    }
}

#[cfg(test)]
#[path = "main_test.rs"]
mod tests;
