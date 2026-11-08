//! Verification of the engine's SHA-256 receipt.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use shipdeck_protocol::message::agent::UnpackRelease;

const MAX_RECEIPT: u64 = 64 * 1024 * 1024;

/// Checks regular files listed in the receipt without following links.
pub(super) fn check(request: &UnpackRelease, staging: &Path) -> Result<Vec<String>> {
    let path = Path::new(&request.root)
        .join("releases")
        .join(format!(".{}.sha256", request.release));
    let metadata = fs::symlink_metadata(&path).context("reading the release receipt")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_RECEIPT {
        bail!("invalid release receipt");
    }
    let mut mismatches = Vec::new();
    let mut seen = HashSet::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        let (hash, name) = line.split_once("  ./").context("invalid receipt entry")?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("invalid receipt checksum");
        }
        let relative = crate::unpack::safe_relative(Path::new(name))?;
        if !seen.insert(relative.clone()) {
            bail!("duplicate receipt entry: {name}");
        }
        let Some(path) = regular_file(staging, &relative)? else {
            mismatches.push(name.to_string());
            continue;
        };
        let mut file = File::open(path)?;
        let mut digest = Sha256::new();
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if base16ct::lower::encode_string(&digest.finalize()) != hash {
            mismatches.push(name.to_string());
        }
    }
    Ok(mismatches)
}

/// Finds a regular file while rejecting symbolic links in all path components.
pub(super) fn regular_file(root: &Path, relative: &Path) -> Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(None),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("checking {}", path.display())),
        }
    }
    match fs::metadata(&path) {
        Ok(metadata) => Ok(metadata.is_file().then_some(path)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("checking {}", path.display())),
    }
}
