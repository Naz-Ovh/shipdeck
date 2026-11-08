//! Copies unchanged files from the current release into staging.

use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::agent::UnpackRelease;

const MAX_LIST: u64 = 64 * 1024 * 1024;

/// Copies the listed files and reports any that can no longer be reused.
pub(super) fn copy(
    request: &UnpackRelease,
    staging: &Path,
    mut size: u64,
    mut entries: u64,
) -> Result<(u64, u64, Vec<String>)> {
    let root = Path::new(&request.root);
    let current = if request.modes.is_empty() {
        None
    } else {
        match fs::canonicalize(root.join("current")) {
            Ok(path) => Some(path),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err).context("opening the current release"),
        }
    };
    let mut mismatches = Vec::new();
    let mut list_bytes = 0_u64;
    for mode in &request.modes {
        let list = root
            .join("releases")
            .join(format!(".{}.reuse.{mode:04o}", request.release));
        let metadata = fs::symlink_metadata(&list).context("reading a reuse list")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_LIST {
            bail!("invalid reuse list");
        }
        list_bytes = list_bytes
            .checked_add(metadata.len())
            .context("reuse list size overflow")?;
        if list_bytes > MAX_LIST {
            bail!("reuse lists are too large");
        }
        let bytes = fs::read(list)?;
        if !bytes.is_empty() && !bytes.ends_with(&[0]) {
            bail!("reuse list is incomplete");
        }
        for raw in bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
        {
            entries = entries
                .checked_add(1)
                .context("release entry count overflow")?;
            if entries > request.max_entries {
                bail!("too many release entries");
            }
            let name = std::str::from_utf8(raw)?
                .strip_prefix("./")
                .context("invalid reuse path")?;
            let relative = crate::unpack::safe_relative(Path::new(name))?;
            let source = match &current {
                Some(current) => crate::unpack_verify::regular_file(current, &relative)?,
                None => None,
            };
            let Some(source) = source else {
                mismatches.push(name.to_string());
                continue;
            };
            let metadata = match fs::metadata(&source) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    mismatches.push(name.to_string());
                    continue;
                }
                Err(err) => return Err(err).context("checking a reusable file"),
            };
            let Some(next) = size.checked_add(metadata.len()) else {
                bail!("release size overflow")
            };
            if next > request.max_bytes {
                mismatches.push(name.to_string());
                continue;
            }
            let destination = prepare_parent(staging, &relative)?;
            if copy_one(&source, &destination, *mode, metadata.len())? {
                size = next;
            } else {
                mismatches.push(name.to_string());
            }
        }
    }
    Ok((size, entries, mismatches))
}

/// Creates parent directories without traversing symbolic links.
fn prepare_parent(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    if let Some(parent) = relative.parent() {
        for part in parent.components() {
            path.push(part.as_os_str());
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => bail!("reuse parent is not a directory: {}", path.display()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => fs::create_dir(&path)?,
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(root.join(relative))
}

fn copy_one(source: &Path, destination: &Path, mode: u32, length: u64) -> Result<bool> {
    let mut source = match File::open(source) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).context("opening a reusable file"),
    };
    let metadata = source.metadata()?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let limit = length.checked_add(1).context("file size overflow")?;
    let copied = io::copy(&mut (&mut source).take(limit), &mut destination)?;
    if copied != length {
        return Ok(false);
    }
    destination.set_times(FileTimes::new().set_modified(metadata.modified()?))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        destination.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(true)
}
