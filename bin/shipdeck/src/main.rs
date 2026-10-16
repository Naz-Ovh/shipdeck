//! shipdeck: build on this machine, deploy over SSH, roll back any release.
//! The flags pick the terminal UI (`shipdeck-tui`) or a command run without
//! it (`shipdeck-headless`); both drive `shipdeck-engine`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use shipdeck_engine::config::{self, Discovery, Manager};
use shipdeck_engine::init::Preset;
use shipdeck_engine::ssh::{Login, Pool, Question};
use shipdeck_headless::{Headless, OnFailure};

mod init;
mod man;

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The password, for runs where `--password` would land in a log.
const PASSWORD_ENV: &str = "SHIPDECK_PASSWORD";
/// A private key's text, such as a CI secret, kept out of `ps` and logs.
const KEY_ENV: &str = "SHIPDECK_SSH_KEY";
/// The passphrase of an encrypted key, so nothing is asked.
const PASSPHRASE_ENV: &str = "SHIPDECK_KEY_PASSPHRASE";

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

    /// Log in with this private key's text (it shows in `ps`; SHIPDECK_SSH_KEY does not)
    #[arg(long, value_name = "TEXT", conflicts_with = "password")]
    key_text: Option<String>,

    /// Log in with this password (it lands in your shell history; SHIPDECK_PASSWORD does not)
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,

    /// Run server commands without the login shell's startup files (~/.bashrc, ~/.zshrc, ...), on every server
    #[arg(long)]
    no_shell_env: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Create a deployable folder with a deploy.yaml from a preset; asks for what the flags leave out
    Init(InitArgs),
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

    /// Trust and record the host key of a server not yet in ~/.ssh/known_hosts; a changed key is still refused
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
        Some(Command::Server(command)) => command,
        None => {
            let root = cli
                .root
                .expect("clap requires DEPLOY_ROOT without --schema or a command");
            let shell_env = !cli.login.no_shell_env;
            return run_tui(root, login(cli.login)?, shell_env).await;
        }
    };

    let (root, ssh) = match &command {
        ServerCommand::Deploy { root, ssh, .. }
        | ServerCommand::Rollback { root, ssh, .. }
        | ServerCommand::Releases { root, ssh, .. } => (root.as_path(), ssh),
    };
    let shell_env = !ssh.login.no_shell_env;
    let login = login(ssh.login.clone())?;
    let Some(discovery) = discover(root)? else {
        return Ok(ExitCode::FAILURE);
    };
    let (questions_tx, questions) = unbounded_channel();
    let pool = Arc::new(pool(login, questions_tx, shell_env));
    let mut headless = Headless::new(pool.clone(), questions, ssh.trust_new_host);
    let code = match command {
        ServerCommand::Deploy {
            names,
            on_failure,
            allow_unchanged,
            ..
        } => {
            headless
                .deploy(&discovery, &names, on_failure.into(), allow_unchanged)
                .await
        }
        ServerCommand::Rollback { name, to, .. } => headless.rollback(&discovery, &name, to).await,
        ServerCommand::Releases { name, .. } => headless.releases(&discovery, &name).await,
    };
    pool.close_all().await;
    Ok(code)
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

async fn run_tui(root: PathBuf, login: Login, shell_env: bool) -> Result<ExitCode> {
    let Some(discovery) = discover(&root)? else {
        return Ok(ExitCode::FAILURE);
    };
    let (questions_tx, questions) = unbounded_channel();
    let (ops_tx, ops) = unbounded_channel();
    let (fetched_tx, fetched) = unbounded_channel();
    let pool = Arc::new(pool(login, questions_tx, shell_env));
    let app = shipdeck_tui::App::new(root, discovery, pool, ops_tx, fetched_tx);
    shipdeck_tui::run(
        app,
        shipdeck_tui::Inbox {
            questions,
            ops,
            fetched,
        },
    )
    .await?;
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

/// The session pool for this run, with the passphrase and shell settings
/// from the environment and the flags.
fn pool(login: Login, asker: UnboundedSender<Question>, shell_env: bool) -> Pool {
    let pool = Pool::new(login, asker).with_passphrase(passphrase());
    if shell_env {
        pool
    } else {
        pool.without_shell_env()
    }
}

fn passphrase() -> Option<String> {
    std::env::var(PASSPHRASE_ENV).ok().filter(|p| !p.is_empty())
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
