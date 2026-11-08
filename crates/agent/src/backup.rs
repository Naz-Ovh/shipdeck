//! A release's backup, packed and unpacked natively with tar and zstd.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{
    AgentBackupCreate, AgentBackupCreated, AgentBackupRestore, AgentOk,
};

/// How many files one backup may hold.
const MAX_ENTRIES: u64 = 1_000_000;
/// How large one unpacked backup may be.
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Packs the chosen release paths into `backups/<release>.tar.zst`.
pub(crate) async fn create(request: AgentBackupCreate) -> Result<AgentBackupCreated> {
    let token = crate::transfer::random_token().await?;
    tokio::task::spawn_blocking(move || create_sync(&request, &token)).await?
}

/// Unpacks `backups/<release>.tar.zst` into `releases/<release>` again.
pub(crate) async fn restore(request: AgentBackupRestore) -> Result<AgentOk> {
    tokio::task::spawn_blocking(move || restore_sync(&request)).await?
}

fn create_sync(request: &AgentBackupCreate, token: &str) -> Result<AgentBackupCreated> {
    let root = crate::paths::absolute_root(&request.root)?;
    let release = crate::paths::real_release(&root, &request.release)?;
    if u64::try_from(request.paths.len())? > MAX_ENTRIES {
        bail!("the backup has too many files");
    }
    let backups = crate::paths::real_folder(&root, "backups")?;
    let done = backups.join(format!("{}.tar.zst", request.release));
    if fs::symlink_metadata(&done).is_ok() {
        bail!("the backup already exists");
    }
    let prefix = format!(".{}.", request.release);
    for entry in fs::read_dir(&backups).context("reading the backups directory")? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(&prefix) && crate::files::ends_with(name, ".part") {
            bail!(
                "an unfinished backup of {} is there; inspect it before cleanup",
                request.release
            );
        }
    }
    let part = backups.join(format!(".{}.{token}.part", request.release));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&part).context("creating the backup")?;
    let encoder = zstd::stream::write::Encoder::new(file, 0).context("starting zstd")?;
    let mut tar = tar::Builder::new(encoder);
    let mut seen = std::collections::HashSet::new();
    for path in &request.paths {
        let relative = safe_path(path)?;
        if !seen.insert(relative.clone()) {
            continue;
        }
        let full = existing(&release, &relative)?;
        let name = Path::new(".").join(&relative);
        tar.append_path_with_name(&full, &name)
            .with_context(|| format!("adding {}", relative.display()))?;
    }
    let encoder = tar.into_inner().context("finishing the backup")?;
    let file = encoder.finish().context("finishing zstd")?;
    file.sync_all().context("saving the backup")?;
    drop(file);
    publish_backup(&part, &done, &request.release)?;
    crate::owner::apply(&done).context("giving the backup to the deploy user")?;
    let bytes = fs::metadata(&done)
        .context("reading the backup size")?
        .len();
    Ok(AgentBackupCreated { bytes })
}

/// Publishes a packed backup; on a non-duplicate failure the part is kept.
fn publish_backup(part: &Path, done: &Path, release: &str) -> Result<()> {
    if let Err(err) = fs::hard_link(part, done) {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            // A finished duplicate is not interrupted work: remove only our file.
            fs::remove_file(part).context("removing the duplicate backup's temporary name")?;
            bail!("a backup of {release} appeared while this one was packed");
        }
        return Err(err).with_context(|| {
            format!(
                "publishing the backup; the packed backup is kept at {}",
                part.display()
            )
        });
    }
    fs::remove_file(part).context("removing the finished backup's temporary name")?;
    Ok(())
}

fn restore_sync(request: &AgentBackupRestore) -> Result<AgentOk> {
    let root = crate::paths::absolute_root(&request.root)?;
    let releases = crate::paths::real_folder(&root, "releases")?;
    let done = releases.join(&request.release);
    if crate::unpack::entry_exists(&done)? {
        return Ok(AgentOk {});
    }
    let backup = crate::paths::real_backup(&root, &request.release)?;
    let staging = releases.join(format!(".{}.restore.partial", request.release));
    if crate::unpack::entry_exists(&staging)? {
        bail!(
            "an unfinished restore of {} is already there; inspect it before cleanup",
            request.release
        );
    }
    fs::create_dir(&staging).context("creating the restore folder")?;
    let result = extract(&backup, &staging);
    if let Err(err) = result {
        return Err(err.context("the unfinished restore folder was kept for inspection"));
    }
    if crate::unpack::entry_exists(&done)? {
        bail!("the release appeared while the backup was unpacked");
    }
    crate::owner::apply_tree(&staging).context("giving the restored release to the deploy user")?;
    fs::rename(&staging, &done).context("finishing the restored release")?;
    Ok(AgentOk {})
}

fn extract(backup: &Path, staging: &Path) -> Result<()> {
    let decoder = zstd::stream::read::Decoder::new(File::open(backup)?)?;
    let mut tar = tar::Archive::new(decoder.take(MAX_BYTES + 1));
    tar.set_preserve_permissions(true);
    let mut seen = std::collections::HashSet::new();
    let mut total = 0_u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = crate::unpack::safe_relative(&entry.path()?)?;
        if !seen.insert(path.clone()) {
            bail!("the backup has a duplicate path: {}", path.display());
        }
        if u64::try_from(seen.len())? > MAX_ENTRIES {
            bail!("the backup has too many entries");
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            bail!("unsupported backup entry: {}", path.display());
        }
        let mode = entry.header().mode()?;
        if mode & 0o7000 != 0 {
            bail!("backup entry has unsafe permissions: {}", path.display());
        }
        if kind.is_symlink() {
            let target = entry.link_name()?.context("symbolic link has no target")?;
            crate::unpack::check_link(&path, &target)?;
        }
        total = total
            .checked_add(entry.size())
            .context("backup size overflow")?;
        if total > MAX_BYTES {
            bail!("the backup is larger than {MAX_BYTES} bytes");
        }
        if !entry.unpack_in(staging)? {
            bail!("backup path escapes the release");
        }
    }
    let mut decoder = tar.into_inner();
    io::copy(&mut decoder, &mut io::sink()).context("checking the compressed backup")?;
    if decoder.limit() == 0 {
        bail!("the decompressed backup is too large");
    }
    Ok(())
}

/// A path from the engine, kept relative and inside the release.
fn safe_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() {
        bail!("empty release path");
    }
    let mut result = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(name) => result.push(name),
            Component::CurDir => {}
            _ => bail!("unsafe release path: {path}"),
        }
    }
    if result.as_os_str().is_empty() {
        bail!("empty release path");
    }
    Ok(result)
}

/// Finds `relative` under `release` without following a link in its parents.
fn existing(release: &Path, relative: &Path) -> Result<PathBuf> {
    let mut path = release.to_path_buf();
    let count = relative.components().count();
    for (index, part) in relative.components().enumerate() {
        let Component::Normal(name) = part else {
            bail!("unsafe release path");
        };
        path.push(name);
        let metadata =
            fs::symlink_metadata(&path).with_context(|| format!("reading {}", path.display()))?;
        if index + 1 < count && metadata.file_type().is_symlink() {
            bail!("{} is behind a symbolic link", relative.display());
        }
    }
    Ok(path)
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
