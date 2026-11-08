//! Safe extraction of a verified release archive into a staging directory.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::{AgentEventAppend, UnpackRelease, UnpackResult};

const MAX_ENTRIES: u64 = 1_000_000;
const MAX_PATH_BYTES: u64 = 128 * 1024 * 1024;

/// Unpacks on a blocking worker while the control channel stays responsive.
pub(crate) async fn unpack(request: UnpackRelease) -> Result<UnpackResult> {
    let token = crate::transfer::random_token().await?;
    record(&request, "unpack_started", "").await?;
    let project = request.root.clone();
    let release = request.release.clone();
    let result = tokio::task::spawn_blocking(move || unpack_sync(&request, &token))
        .await
        .context("unpack worker stopped")?;
    let (kind, detail) = match &result {
        Ok(result) if result.verified => ("unpack_verified", String::new()),
        Ok(result) => (
            "unpack_mismatch",
            format!("{} files did not match", result.mismatches.len()),
        ),
        Err(err) => ("unpack_failed", format!("{err:#}")),
    };
    crate::state::append(AgentEventAppend {
        project,
        operation: release,
        kind: kind.to_string(),
        detail,
    })
    .await
    .context("saving the unpack result")?;
    result
}

async fn record(request: &UnpackRelease, kind: &str, detail: &str) -> Result<()> {
    crate::state::append(AgentEventAppend {
        project: request.root.clone(),
        operation: request.release.clone(),
        kind: kind.to_string(),
        detail: detail.to_string(),
    })
    .await
    .context("saving the unpack event")?;
    Ok(())
}

fn unpack_sync(request: &UnpackRelease, token: &str) -> Result<UnpackResult> {
    let root = Path::new(&request.root);
    if !root.is_absolute() || !crate::transfer::safe_id(&request.release) {
        bail!("invalid deployment root or release name");
    }
    if request.max_entries > MAX_ENTRIES {
        bail!("the release has more than {MAX_ENTRIES} entries");
    }
    let mut modes = HashSet::new();
    if request
        .modes
        .iter()
        .any(|mode| *mode > 0o777 || !modes.insert(*mode))
    {
        bail!("invalid or duplicate file mode");
    }
    let releases = root.join("releases");
    let finished = releases.join(&request.release);
    if entry_exists(&finished)? {
        bail!("the release already exists");
    }
    let archive = releases.join(format!(".{}.tar.zst", request.release));
    let archive_meta = fs::symlink_metadata(&archive).context("checking the uploaded archive")?;
    if !archive_meta.is_file() || archive_meta.file_type().is_symlink() {
        bail!("the uploaded archive must be a regular file");
    }
    let held = releases.join(format!(".{}.{token}.archive.partial", request.release));
    let staging = releases.join(format!(".{}.{token}.partial", request.release));
    fs::rename(&archive, &held).context("claiming the uploaded archive")?;
    fs::create_dir(&staging).context("creating release staging")?;
    let (size, entries) = extract(&held, &staging, request.max_bytes, request.max_entries)?;
    let (size, entries, mut mismatches) =
        crate::unpack_reuse::copy(request, &staging, size, entries)?;
    mismatches.extend(crate::unpack_verify::check(request, &staging)?);
    mismatches.sort();
    mismatches.dedup();
    if !mismatches.is_empty() {
        return Ok(UnpackResult {
            verified: false,
            mismatches,
        });
    }
    if size != request.max_bytes {
        bail!("unpacked size does not match the release manifest");
    }
    if entries != request.max_entries {
        bail!("unpacked file count does not match the release manifest");
    }
    if entry_exists(&finished)? {
        bail!("the release appeared while staging was prepared");
    }
    crate::owner::apply_tree(&staging).context("giving the release to the deploy user")?;
    fs::rename(&staging, &finished).context("finishing the release")?;
    fs::remove_file(&held).context("removing the completed archive")?;
    fs::remove_file(releases.join(format!(".{}.sha256", request.release)))
        .context("removing the completed receipt")?;
    for mode in &request.modes {
        fs::remove_file(releases.join(format!(".{}.reuse.{mode:04o}", request.release)))
            .context("removing a completed reuse list")?;
    }
    Ok(UnpackResult {
        verified: true,
        mismatches,
    })
}

fn extract(archive: &Path, staging: &Path, max_bytes: u64, max_entries: u64) -> Result<(u64, u64)> {
    let decoder = zstd::stream::read::Decoder::new(File::open(archive)?)?;
    let limit = max_entries
        .checked_mul(16 * 1024)
        .and_then(|bytes| bytes.checked_add(max_bytes))
        .and_then(|bytes| bytes.checked_add(1024 * 1024 + 1))
        .context("archive limit overflow")?;
    let mut tar = tar::Archive::new(decoder.take(limit));
    tar.set_preserve_permissions(true);
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    let mut path_bytes = 0_u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = safe_relative(&entry.path()?)?;
        path_bytes = path_bytes
            .checked_add(u64::try_from(path.as_os_str().len())?)
            .context("archive path size overflow")?;
        if path_bytes > MAX_PATH_BYTES {
            bail!("archive paths are too large");
        }
        if !seen.insert(path.clone()) {
            bail!("archive contains a duplicate path: {}", path.display());
        }
        if u64::try_from(seen.len())? > max_entries {
            bail!("archive contains more entries than declared");
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            bail!("unsupported archive entry: {}", path.display());
        }
        if entry.header().mode()? & 0o7000 != 0 {
            bail!("archive entry has unsafe permissions: {}", path.display());
        }
        if kind.is_symlink() {
            let target = entry.link_name()?.context("symbolic link has no target")?;
            check_link(&path, &target)?;
        }
        total = total
            .checked_add(entry.size())
            .context("release size overflow")?;
        if total > max_bytes {
            bail!("archive exceeds the declared release size");
        }
        if !entry.unpack_in(staging)? {
            bail!("archive path escapes the release");
        }
    }
    let mut decoder = tar.into_inner();
    io::copy(&mut decoder, &mut io::sink()).context("checking the compressed stream")?;
    if decoder.limit() == 0 {
        bail!("decompressed archive exceeds the declared limit");
    }
    Ok((total, u64::try_from(seen.len())?))
}

pub(super) fn entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// Normalizes a relative path and rejects any escape.
pub(super) fn safe_relative(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => result.push(name),
            Component::CurDir => {}
            _ => bail!("unsafe release path: {}", path.display()),
        }
    }
    if result.as_os_str().is_empty() {
        bail!("release path is empty");
    }
    Ok(result)
}

pub(super) fn check_link(path: &Path, target: &Path) -> Result<()> {
    let mut depth = path
        .parent()
        .map_or(0, |parent| parent.components().count());
    if target.as_os_str().is_empty() {
        bail!("symbolic link target is empty");
    }
    for part in target.components() {
        match part {
            Component::Normal(_) => depth = depth.checked_add(1).context("link depth overflow")?,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => bail!("symbolic link escapes the release: {}", path.display()),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "unpack_tests.rs"]
mod tests;
