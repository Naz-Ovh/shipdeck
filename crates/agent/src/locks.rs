//! Project locks live only as long as this agent connection.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{AgentOk, ProjectLock};

#[derive(Default)]
pub(crate) struct ProjectLocks(HashMap<PathBuf, (u64, File)>);

impl ProjectLocks {
    /// Exclusively locks an existing project until release or disconnect.
    pub(crate) fn acquire(&mut self, request: &ProjectLock) -> Result<AgentOk> {
        let root = canonical_root(&request.root)?;
        if self.0.contains_key(&root) {
            bail!("another operation in this session already holds the project lock");
        }
        let path = root.join(".shipdeck.lock");
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            bail!("the project lock cannot be a symbolic link");
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).context("opening the project lock")?;
        file.try_lock()
            .context("another operation is using this project")?;
        // The receiver, on its own process, reads this token to prove that
        // the upload still belongs to the operation holding the lock.
        file.set_len(0).context("writing the project lock token")?;
        write!(file, "{}", request.token).context("writing the project lock token")?;
        file.flush().context("writing the project lock token")?;
        self.0.insert(root, (request.token, file));
        Ok(AgentOk {})
    }

    /// The token of this connection's lock on the project, if held.
    pub(crate) fn token(&self, root: &str) -> Option<u64> {
        let root = canonical_root(root).ok()?;
        self.0.get(&root).map(|(token, _)| *token)
    }

    /// Whether this connection holds the project's lock.
    pub(crate) fn holds(&self, root: &str) -> bool {
        canonical_root(root).is_ok_and(|root| self.0.contains_key(&root))
    }

    /// Releases this connection's lock on a project.
    pub(crate) fn release(&mut self, request: &ProjectLock) -> Result<AgentOk> {
        let root = canonical_root(&request.root)?;
        if self
            .0
            .get(&root)
            .is_none_or(|(token, _)| *token != request.token)
        {
            bail!("this session does not hold the project lock");
        }
        self.0.remove(&root);
        Ok(AgentOk {})
    }
}

fn canonical_root(root: &str) -> Result<PathBuf> {
    if !Path::new(root).is_absolute() {
        bail!("deployment root must be absolute");
    }
    let root = std::fs::canonicalize(root).context("opening the project directory")?;
    if !root.is_dir() {
        bail!("deployment root is not a directory");
    }
    Ok(root)
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
