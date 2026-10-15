//! shipdeck: build on this machine, deploy over SSH, roll back any release.
//! The flags, the login and the config check; the screen is `shipdeck-tui`.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::sync::mpsc::unbounded_channel;

use shipdeck_engine::config;
use shipdeck_engine::ssh::{Login, Pool};

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "shipdeck",
    version,
    about = "Build locally, deploy over SSH as a systemd service, roll back any release.",
    after_help = "Each folder under DEPLOY_ROOT with a deploy.yaml is one deployable.\n\
                  With neither --key nor --password, the host's ~/.ssh/config key is tried,\n\
                  then the TUI asks for the password."
)]
struct Cli {
    /// Folder whose subfolders each hold a deploy.yaml
    #[arg(value_name = "DEPLOY_ROOT", required_unless_present = "schema")]
    root: Option<PathBuf>,

    /// Log in with this private key file
    #[arg(long, value_name = "PATH", conflicts_with = "password")]
    key: Option<PathBuf>,

    /// Log in with this password (it lands in your shell history)
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,

    /// Print the JSON Schema of deploy.yaml, for editor completion
    #[arg(long)]
    schema: bool,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    if cli.schema {
        print!("{}", config::schema::text());
        return Ok(ExitCode::SUCCESS);
    }
    let root = cli
        .root
        .expect("clap requires DEPLOY_ROOT without --schema");
    if !root.is_dir() {
        bail!("{} is not a folder", root.display());
    }
    let login = match (cli.key, cli.password) {
        (Some(key), _) => {
            let key = expand_home(key);
            if !key.is_file() {
                bail!("the key file {} does not exist", key.display());
            }
            Login::Key(key)
        }
        (None, Some(password)) => Login::Password(password),
        (None, None) => Login::Auto,
    };
    let discovery = config::discover(&root)
        .with_context(|| format!("looking for deployables in {}", root.display()))?;
    if !discovery.problems.is_empty() {
        // Every problem in every file at once, then no TUI: a deploy from a
        // config shipdeck cannot fully read is never offered.
        let base = std::env::current_dir().unwrap_or_default();
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        eprint!("{}", config::render(&discovery.problems, &base, color));
        return Ok(ExitCode::FAILURE);
    }

    let (questions_tx, questions) = unbounded_channel();
    let (ops_tx, ops) = unbounded_channel();
    let (fetched_tx, fetched) = unbounded_channel();
    let pool = Arc::new(Pool::new(login, questions_tx));
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

/// `~/.ssh/id_ed25519` → `/home/me/.ssh/id_ed25519`; zsh expands `~` only
/// at the start of a word, not after `--key=`.
fn expand_home(path: PathBuf) -> PathBuf {
    match (path.strip_prefix("~"), std::env::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path,
    }
}
