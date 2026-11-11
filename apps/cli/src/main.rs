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

use shipdeck_engine::config::{self, Env, ManagerKind, ServerInputs};
use shipdeck_engine::init::Preset;
use shipdeck_protocol::message::DeployableInfo;

use console::{Choices, Console};
use naming::{Where, fail, problems, resolve_name, resolve_names, setup_access, setup_password};
use shipdeck_protocol::message::common::OnFailure;
use shipdeck_protocol::message::request::SetupAccess;

mod console;
mod engine;
mod init;
mod login;
mod man;
mod naming;
mod target;

use login::{LoginArgs, gitshell, login, wire};
use target::Target;

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "shipdeck",
    version,
    about = "Build locally, deploy over SSH as a systemd service or PM2 app, roll back any release.",
    after_help = "shipdeck reads the root file, shipdeck.yaml or .shipdeck, in the project folder;\n\
                  -C moves that folder and -c names another root file. A deploy is one file:\n\
                  names come from the root file, and a .yaml or .yml argument is a deploy file.\n\
                  Login, first found: --passwordless, --key-file, --key-text, --password, SHIPDECK_SSH_KEY,\n\
                  SHIPDECK_PASSWORD; else the host's ~/.ssh/config key, then the TUI asks\n\
                  for the password. SHIPDECK_KEY_PASSPHRASE unlocks an encrypted key.\n\
                  Server, for one deploy: -H/--host, -P/--port, -U/--user, else SHIPDECK_SSH_HOST,\n\
                  SHIPDECK_SSH_PORT, SHIPDECK_SSH_USER, else the file; `${NAME}` in server.ssh reads a variable.\n\
                  .env in the project folder (or -E <file>) fills `${NAME}` and the SHIPDECK_* variables above.\n\
                  deploy, rollback and releases run without the TUI and ask nothing.\n\
                  init writes a root file and a deploy file; it asks for what its flags leave out.\n\
                  man is the full manual: `shipdeck man index`, `man configs`, `man details <KEY>...`.",
    subcommand_negates_reqs = true
)]
struct Cli {
    /// Run as if started in this folder; every relative path on the command line starts there
    #[arg(short = 'C', long, value_name = "DIR", global = true)]
    cwd: Option<PathBuf>,

    /// The root file to read, instead of shipdeck.yaml or .shipdeck in the project folder
    #[arg(short = 'c', long, value_name = "FILE", global = true)]
    config: Option<PathBuf>,

    /// Read this env file instead of .env in the project folder; its values never reach your commands
    #[arg(short = 'E', long, value_name = "FILE", global = true)]
    env_file: Option<PathBuf>,

    #[command(flatten)]
    login: LoginArgs,

    /// Print the JSON Schema of the root and deploy files, for editor completion
    #[arg(long)]
    schema: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Write a root file and a deploy file from a preset; asks for what the flags leave out
    Init(InitArgs),
    /// Run as the engine: it answers the app on stdin and stdout, one message per line
    Engine,
    #[command(flatten)]
    Server(ServerCommand),
    /// The manual, for people and coding agents: `man index` (topics), `man configs`, `man details <KEY>...`
    Man(man::ManArgs),
}

/// The commands that log in to a server.
#[derive(Subcommand)]
enum ServerCommand {
    /// Build and deploy one or more deploys, one after another, without the TUI
    Deploy {
        /// Deploys to deploy: names from the root file, or .yaml/.yml files. The first failure stops the rest
        #[arg(value_name = "DEPLOY")]
        names: Vec<Target>,

        /// What to do when the new release fails after the switch; without it, `release.steps.rollback.mode` decides
        #[arg(long, value_enum)]
        on_failure: Option<OnFailureArg>,

        /// Deploy even when nothing changed since the live release; without it, such a deploy stops with exit code 6
        #[arg(long)]
        allow_unchanged: bool,

        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Switch a deploy to a release already on the server, without the TUI
    Rollback {
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Target,

        /// The release id to make live, as `shipdeck releases` lists it
        #[arg(value_name = "RELEASE")]
        release: String,

        #[command(flatten)]
        ssh: SshArgs,
    },
    /// List the releases on the server, newest first
    Releases {
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Option<Target>,

        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Show Linux, CPU, memory, disk, uptime, and installed tools on a server
    ServerInfo {
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Option<Target>,
        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Show what an interrupted operation left, and clean it up with --cleanup
    Work {
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Option<Target>,
        /// Remove the unfinished files this command lists; this flag is the approval
        #[arg(long)]
        cleanup: bool,
        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Print the events saved on the server, oldest first
    Events {
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Option<Target>,
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
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Option<Target>,
        /// Password-free sudo for every command, or only the ones shipdeck runs
        #[arg(long, value_enum)]
        access: Option<AccessArg>,
        /// The sudo password for this run; `SHIPDECK_SUDO_PASSWORD` keeps it out of history
        #[arg(long, value_name = "PASSWORD")]
        sudo_password: Option<String>,
        #[command(flatten)]
        ssh: SshArgs,
    },
    /// Print every path the next release will hold; never connects to the server
    Tree {
        /// The deploy: a name from the root file, or a .yaml/.yml file
        #[arg(value_name = "DEPLOY")]
        name: Option<Target>,
        /// Only one app's folder
        #[arg(long, value_name = "APP")]
        app: Option<String>,
        /// Fold folders deeper than this into `name/ (<count> files)`
        #[arg(long, value_name = "N")]
        depth: Option<usize>,
        /// Run the build steps first, so build output is there
        #[arg(long)]
        build: bool,
        /// One object per path: path, kind, from, local, mode, size
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
struct InitArgs {
    /// The deploy's name, also its process name; such as my-api
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

impl From<ManagerArg> for ManagerKind {
    fn from(arg: ManagerArg) -> Self {
        match arg {
            ManagerArg::Systemd => ManagerKind::Systemd,
            ManagerArg::Pm2 => ManagerKind::Pm2,
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
            let root = folder(cli.cwd.as_deref())?;
            let env = Env::load(&root, cli.env_file.as_deref())?;
            return run_tui(&root, cli.config, cli.env_file, cli.login, env).await;
        }
    };
    // `-C` applies first, as in `git -C`: every relative path starts there.
    let root = folder(cli.cwd.as_deref())?;
    let env = Env::load(&root, cli.env_file.as_deref())?;
    let config = cli.config.map(|file| root.join(file));
    let deploy_files: Vec<PathBuf> = targets(&command).map(|target| root.join(target)).collect();
    if let ServerCommand::Tree {
        name,
        app,
        depth,
        build,
        json,
    } = &command
    {
        return run_tree(
            &root,
            config.as_deref(),
            &ServerInputs::from_env(env),
            name.as_ref(),
            app.as_deref(),
            *depth,
            *build,
            *json,
        )
        .await;
    }
    let ssh = ssh_args(&command);
    let shell_env = !ssh.login.no_shell_env;
    let gitshell = gitshell(&ssh.login);
    let login = login(ssh.login.clone(), &env)?;
    let engine = engine::Engine::start(gitshell.as_deref()).await?;
    let workspace = engine
        .open(engine::Opening {
            root: &root,
            login: &wire(&login),
            no_shell_env: !shell_env,
            config: config.as_deref(),
            deploy_files: &deploy_files,
            env_file: cli.env_file.as_deref(),
            ssh: ssh.login.ssh(),
            run: names(&command),
        })
        .await?;
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
            setup_password(sudo_password.clone(), &env)?,
        )),
        _ => None,
    };
    let (on_failure, allow_unchanged) = match &command {
        ServerCommand::Deploy {
            on_failure,
            allow_unchanged,
            ..
        } => (on_failure.map(OnFailure::from), *allow_unchanged),
        _ => (None, false),
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
    let (names, root_file) = (workspace.deployables, root_file(&root, config.as_deref()));
    let where_ = Where {
        root: &root,
        root_file: &root_file,
    };
    let code = run_command(&mut console, command, &names, &where_, setup).await;
    console.close().await;
    let _ = child.wait().await;
    Ok(code)
}

/// The root file a run reads, for a message that names it.
fn root_file(root: &Path, config: Option<&Path>) -> PathBuf {
    if let Some(file) = config {
        return file.to_path_buf();
    }
    config::ROOT_NAMES
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| root.join(config::ROOT_NAMES[0]))
}

/// The project folder: `-C`, or the folder shipdeck was started in.
///
/// # Errors
/// Returns an error when `-C` names a folder that is not there.
fn folder(cwd: Option<&Path>) -> Result<PathBuf> {
    let root = match cwd {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    if !root.is_dir() {
        bail!("{} is not a folder", root.display());
    }
    // Every path this run names starts from here, and every error shows a
    // full path, so the folder is made absolute once and used as it is.
    std::path::absolute(&root).with_context(|| format!("reading {}", root.display()))
}

/// The deploys a command names, in order.
fn named(command: &ServerCommand) -> Vec<Target> {
    match command {
        ServerCommand::Deploy { names, .. } => names.clone(),
        ServerCommand::Rollback { name, .. } => vec![name.clone()],
        ServerCommand::Releases { name, .. }
        | ServerCommand::ServerInfo { name, .. }
        | ServerCommand::Work { name, .. }
        | ServerCommand::Events { name, .. }
        | ServerCommand::Setup { name, .. }
        | ServerCommand::Tree { name, .. } => name.iter().cloned().collect(),
    }
}

/// The deploy files a command names, in order.
fn targets(command: &ServerCommand) -> impl Iterator<Item = PathBuf> {
    named(command)
        .into_iter()
        .filter_map(|target| target.file().map(Path::to_path_buf))
}

/// The root file's deploys a command names, in order.
fn names(command: &ServerCommand) -> Vec<String> {
    named(command)
        .into_iter()
        .filter_map(|target| match target {
            Target::Name(name) => Some(name),
            Target::File(_) => None,
        })
        .collect()
}

/// The SSH flags of a command.
fn ssh_args(command: &ServerCommand) -> &SshArgs {
    match command {
        ServerCommand::Deploy { ssh, .. }
        | ServerCommand::Rollback { ssh, .. }
        | ServerCommand::Releases { ssh, .. }
        | ServerCommand::ServerInfo { ssh, .. }
        | ServerCommand::Work { ssh, .. }
        | ServerCommand::Events { ssh, .. }
        | ServerCommand::Setup { ssh, .. } => ssh,
        ServerCommand::Tree { .. } => unreachable!("tree runs without the server"),
    }
}

/// Runs the one command that was chosen.
async fn run_command(
    console: &mut Console,
    command: ServerCommand,
    projects: &[DeployableInfo],
    where_: &Where<'_>,
    setup: Option<(SetupAccess, Option<String>)>,
) -> ExitCode {
    match command {
        ServerCommand::Deploy { names, .. } => {
            let names = match resolve_names(projects, where_, &names) {
                Ok(names) => names,
                Err(err) => return fail(&err),
            };
            console.deploy(projects, &names).await
        }
        ServerCommand::Rollback { name, release, .. } => {
            let name = match resolve_name(projects, where_, Some(&name)) {
                Ok(name) => name,
                Err(err) => return fail(&err),
            };
            console.rollback(projects, &name, release).await
        }
        ServerCommand::Releases { name, .. } => {
            let name = match resolve_name(projects, where_, name.as_ref()) {
                Ok(name) => name,
                Err(err) => return fail(&err),
            };
            console.releases(projects, &name).await
        }
        ServerCommand::ServerInfo { name, .. } => {
            let name = match resolve_name(projects, where_, name.as_ref()) {
                Ok(name) => name,
                Err(err) => return fail(&err),
            };
            console.server_info(projects, &name).await
        }
        ServerCommand::Work { name, cleanup, .. } => {
            let name = match resolve_name(projects, where_, name.as_ref()) {
                Ok(name) => name,
                Err(err) => return fail(&err),
            };
            console.work(projects, &name, cleanup).await
        }
        ServerCommand::Events {
            name, after, limit, ..
        } => {
            let name = match resolve_name(projects, where_, name.as_ref()) {
                Ok(name) => name,
                Err(err) => return fail(&err),
            };
            console.events(projects, &name, after, limit).await
        }
        ServerCommand::Setup { name, .. } => {
            let name = match resolve_name(projects, where_, name.as_ref()) {
                Ok(name) => name,
                Err(err) => return fail(&err),
            };
            let Some((access, password)) = setup else {
                unreachable!("setup answers were read above");
            };
            console.setup(projects, &name, access, password).await
        }
        ServerCommand::Tree { .. } => unreachable!("tree runs without the engine"),
    }
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
    let root = std::env::current_dir().context("reading the current folder")?;
    let given = init::Given {
        root,
        name: args.name,
        preset: args.preset,
        manager: args.manager.map(Into::into),
        host: args.host,
        user: args.user,
        project: args.project,
    };
    init::run(given)
}

async fn run_tui(
    root: &Path,
    config: Option<PathBuf>,
    env_file: Option<PathBuf>,
    args: LoginArgs,
    env: Env,
) -> Result<ExitCode> {
    let login = login(args.clone(), &env)?;
    let engine = engine::Engine::start(gitshell(&args).as_deref()).await?;
    let workspace = engine
        .open(engine::Opening {
            root,
            login: &wire(&login),
            no_shell_env: !args.no_shell_env,
            config: config.as_deref(),
            deploy_files: &[],
            env_file: env_file.as_deref(),
            ssh: args.ssh(),
            run: Vec::new(),
        })
        .await?;
    if !workspace.problems.is_empty() {
        return Ok(problems(&workspace.problems));
    }
    let (deployables, failures) = config::open_all(
        root,
        config.as_deref(),
        &[],
        &ServerInputs::new(env, args.ssh(), Vec::new()),
    );
    if let Some(found) = failures.first() {
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        eprint!(
            "{}",
            config::render(
                std::slice::from_ref(found),
                root,
                color,
                "shipdeck did not start"
            )
        );
        return Ok(ExitCode::FAILURE);
    }
    if deployables.is_empty() {
        bail!("there is no deploy to show");
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
    let app = shipdeck_tui::App::new(root.to_path_buf(), deployables, login, handle, fetched_tx);
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

/// Prints the release tree; never connects to the server.
#[allow(clippy::too_many_arguments)]
async fn run_tree(
    root: &Path,
    config: Option<&Path>,
    inputs: &ServerInputs,
    name: Option<&Target>,
    app: Option<&str>,
    depth: Option<usize>,
    build: bool,
    json: bool,
) -> Result<ExitCode> {
    let deployment = tree_deploy(root, config, inputs, name)?;
    let options = shipdeck_engine::tree::Options {
        app: app.map(str::to_string),
        depth,
        build,
        json,
    };
    let outcome = shipdeck_engine::tree::run(&deployment, &options).await?;
    print!("{}", outcome.text);
    Ok(if outcome.missing.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The deploy `tree` reads: the file it names, the root file's deploy, or the
/// only deploy the root file lists.
fn tree_deploy(
    root: &Path,
    config: Option<&Path>,
    inputs: &ServerInputs,
    target: Option<&Target>,
) -> Result<config::Deployment> {
    let rendered = |found: &config::FileProblems| {
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        anyhow::anyhow!(
            "{}",
            config::render(
                std::slice::from_ref(found),
                root,
                color,
                "shipdeck did not start"
            )
        )
    };
    // `tree` shows one deploy, so a `SHIPDECK_SSH_*` variable applies to it.
    let details = inputs.details(
        inputs
            .single(1)
            .map_err(|message| anyhow::anyhow!(message))?,
    );
    if let Some(file @ Target::File(_)) = target {
        let Target::File(path) = file.resolved(root) else {
            unreachable!("a deploy file was named");
        };
        return config::load_deploy(root, &path, None, details).map_err(|found| rendered(&found));
    }
    let root_file = config::Root::read(root, config).map_err(|found| rendered(&found))?;
    let name = match target {
        Some(Target::Name(name)) => name.as_str(),
        Some(Target::File(_)) => unreachable!("a deploy file was read above"),
        None => match root_file.deploys.keys().collect::<Vec<_>>().as_slice() {
            [one] => one.as_str(),
            many => {
                bail!(
                    "name the deploy: {}",
                    many.iter()
                        .map(|name| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        },
    };
    if !root_file.deploys.contains_key(name) {
        bail!(
            "{} has no deploy named {name}; it lists {}. A deploy file's name ends in .yaml or .yml",
            root_file.file.display(),
            listed(root_file.deploys.keys())
        );
    }
    root_file
        .deployment(name, details)
        .map_err(|found| rendered(&found))
}

/// The names a message lists, or a word for none.
fn listed<'a>(names: impl Iterator<Item = &'a String>) -> String {
    let names: Vec<&str> = names.map(String::as_str).collect();
    if names.is_empty() {
        "no deploys".into()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
