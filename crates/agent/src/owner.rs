//! Deployment files keep the SSH user's ownership even though the agent now
//! runs as root. Root owns the agent; the deploy user owns app files.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::identity::{self, Account};

/// The user deploying: the SSH account, passed by the engine at launch.
///
/// # Errors
///
/// Fails when the account is missing or a non-root agent names another user.
pub(crate) fn operator() -> Result<Account> {
    let name = std::env::var("SHIPDECK_OPERATOR")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| std::env::var("SUDO_USER").ok().filter(|n| !n.is_empty()))
        .unwrap_or(identity::current_user()?);
    let account = identity::lookup(&name)?;
    if !identity::is_root() {
        let current = identity::current_user()?;
        if current != account.name {
            bail!("this agent cannot give deployment files to {name}; it is not root");
        }
    }
    Ok(account)
}

/// Gives one directory or file to the deploy user.
pub(crate) fn apply(path: &Path) -> Result<()> {
    let account = operator()?;
    change(path, &account)
}

/// Gives a whole unpacked tree to the deploy user without following links.
pub(crate) fn apply_tree(path: &Path) -> Result<()> {
    let account = operator()?;
    walk(path, &account)
}

fn walk(path: &Path, account: &Account) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in
            std::fs::read_dir(path).with_context(|| format!("reading {}", path.display()))?
        {
            walk(&entry?.path(), account)?;
        }
    }
    change(path, account)
}

fn change(path: &Path, account: &Account) -> Result<()> {
    if !identity::is_root() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::chown(path, Some(account.uid), Some(account.gid))
            .with_context(|| format!("giving {} to {}", path.display(), account.name))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, account);
        bail!("file ownership needs a Linux server")
    }
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
