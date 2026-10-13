//! shipdeck: build on this machine, deploy over SSH, roll back any release.

mod tui;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::sync::mpsc::unbounded_channel;

use shipdeck_engine::config;
use shipdeck_engine::ssh::{Login, Pool};

#[derive(Parser)]
#[command(
    name = "shipdeck",
    version,
    about = "Build locally, deploy over SSH as a systemd service, roll back any release.",
    after_help = "Each folder under DEPLOY_ROOT with a deploy.json is one deployable.\n\
                  With neither --key nor --password, the password is asked in the TUI."
)]
struct Cli {
    /// Folder whose subfolders each hold a deploy.json
    #[arg(value_name = "DEPLOY_ROOT")]
    root: PathBuf,

    /// Log in with this private key file
    #[arg(long, value_name = "PATH", conflicts_with = "password")]
    key: Option<PathBuf>,

    /// Log in with this password (it lands in your shell history)
    #[arg(long, value_name = "PASSWORD")]
    password: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if !cli.root.is_dir() {
        bail!("{} is not a folder", cli.root.display());
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
    let discovery = config::discover(&cli.root)
        .with_context(|| format!("looking for deployables in {}", cli.root.display()))?;

    let (questions_tx, questions) = unbounded_channel();
    let (ops_tx, ops) = unbounded_channel();
    let (fetched_tx, fetched) = unbounded_channel();
    let pool = Arc::new(Pool::new(login, questions_tx));
    let app = tui::App::new(cli.root, discovery, pool, ops_tx, fetched_tx);
    tui::run(
        app,
        tui::Inbox {
            questions,
            ops,
            fetched,
        },
    )
    .await
}

/// `~/.ssh/id_ed25519` → `/home/me/.ssh/id_ed25519`; zsh expands `~` only
/// at the start of a word, not after `--key=`.
fn expand_home(path: PathBuf) -> PathBuf {
    match (path.strip_prefix("~"), std::env::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path,
    }
}
