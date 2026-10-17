//! Looking inside a release without changing anything: every file it holds
//! and the text of one, from its folder in `releases/` or from inside its
//! backup, which is read as a stream and never unpacked.

use anyhow::{Result, bail};

use super::{Server, ensure_id};
use crate::release::Manifest;
use crate::ssh::sh_quote;

/// How much of a file is read to show it.
pub const READ_CAP: u64 = 512 * 1024;

/// Where a release's files are read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `releases/<id>/`.
    Folder,
    /// `backups/<id>.tar.zst`.
    Backup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Dir,
    File,
    Link,
}

/// One path in a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub kind: FileKind,
    /// As `ls -l` shows it, such as `-rw-r--r--`.
    pub mode: String,
    pub size: u64,
    /// Where a link points.
    pub target: Option<String>,
}

/// What a release holds.
#[derive(Debug, Clone, Default)]
pub struct ReleaseContents {
    pub files: Vec<FileInfo>,
    /// Its `release.json`, when read from a backup.
    pub manifest: Option<Manifest>,
}

/// The start of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileText {
    /// At most [`READ_CAP`] bytes, as text.
    pub text: String,
    /// It holds a NUL byte, so it is not shown as text.
    pub binary: bool,
}

impl Server {
    /// Every path of release `id` in `source`. A backup is listed by `tar`
    /// as it decompresses, and its `release.json` read the same way.
    pub async fn contents(&self, id: &str, source: Source) -> Result<ReleaseContents> {
        ensure_id(id)?;
        let script = match source {
            Source::Folder => format!(
                "set -e
cd {dir}/releases/{id}
find . -mindepth 1 -printf '%M\\t%s\\t%P\\t%l\\0'",
                dir = self.q_dir()
            ),
            Source::Backup => format!(
                "set -e
cd {dir}
tar -I zstd -xOf backups/{id}.tar.zst --occurrence=1 ./release.json 2>/dev/null | tr -d '\\n\\t'
echo
tar -I zstd -tvf backups/{id}.tar.zst",
                dir = self.q_dir()
            ),
        };
        let out = self
            .session
            .exec(&script)
            .await?
            .check(&format!("listing the files of {id}"))?;
        Ok(match source {
            Source::Folder => ReleaseContents {
                files: parse_find(&out.stdout),
                manifest: None,
            },
            Source::Backup => {
                let (json, listing) = out.stdout.split_once('\n').unwrap_or_default();
                ReleaseContents {
                    files: parse_tar_listing(listing),
                    manifest: serde_json::from_str(json).ok(),
                }
            }
        })
    }

    /// The first [`READ_CAP`] bytes of `path` in release `id`.
    pub async fn read(&self, id: &str, source: Source, path: &str) -> Result<FileText> {
        ensure_id(id)?;
        if path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|part| part == "..")
            || path.contains('\0')
        {
            bail!("refusing an unexpected file name: {path:?}");
        }
        let member = sh_quote(&format!("./{path}"));
        let script = match source {
            Source::Folder => format!(
                "cd {dir}/releases/{id} && head -c {READ_CAP} -- {member}",
                dir = self.q_dir()
            ),
            // tar stops at the first match; head may close the pipe early.
            Source::Backup => format!(
                "cd {dir} || exit 1
tar -I zstd -xOf backups/{id}.tar.zst --occurrence=1 -- {member} | head -c {READ_CAP}",
                dir = self.q_dir()
            ),
        };
        let out = self.session.exec(&script).await?;
        if out.stdout.is_empty() && !out.stderr.trim().is_empty() {
            bail!("reading {path}: {}", out.stderr.trim());
        }
        Ok(FileText {
            binary: out.stdout.contains('\0'),
            text: out.stdout,
        })
    }
}

/// Reads `find -printf '%M\t%s\t%P\t%l\0'`: mode, size, path, link target.
pub(super) fn parse_find(text: &str) -> Vec<FileInfo> {
    let mut files: Vec<FileInfo> = text
        .split('\0')
        .filter_map(|record| {
            let mut fields = record.splitn(4, '\t');
            let (mode, size, path, target) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            if path.is_empty() {
                return None;
            }
            let kind = kind_of(mode);
            Some(FileInfo {
                path: path.to_string(),
                kind,
                mode: mode.to_string(),
                size: size.parse().unwrap_or(0),
                target: (kind == FileKind::Link).then(|| target.to_string()),
            })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Reads GNU `tar -tv` lines: `-rw-r--r-- root/root 12 2026-10-17 18:37 ./a`,
/// a folder's name ending in `/`, a link's in ` -> target`.
pub(super) fn parse_tar_listing(text: &str) -> Vec<FileInfo> {
    let mut files: Vec<FileInfo> = text
        .lines()
        .filter_map(|line| {
            let (mode, rest) = word(line)?;
            let (_owner, rest) = word(rest)?;
            let (size, rest) = word(rest)?;
            let (_date, rest) = word(rest)?;
            let (_time, rest) = word(rest)?;
            let name = rest.strip_prefix(' ')?;
            let kind = kind_of(mode);
            let (name, target) = match name.split_once(" -> ") {
                Some((name, target)) if kind == FileKind::Link => (name, Some(target)),
                _ => (name, None),
            };
            let path = name
                .strip_prefix("./")
                .unwrap_or(name)
                .trim_end_matches('/');
            (!path.is_empty() && path != ".").then(|| FileInfo {
                path: path.to_string(),
                kind,
                mode: mode.to_string(),
                size: size.parse().unwrap_or(0),
                target: target.map(str::to_string),
            })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// The first word of `text` and what follows it, from the space after it.
fn word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches(' ');
    let end = text.find(' ')?;
    Some((&text[..end], &text[end..]))
}

fn kind_of(mode: &str) -> FileKind {
    match mode.chars().next() {
        Some('d') => FileKind::Dir,
        Some('l') => FileKind::Link,
        _ => FileKind::File,
    }
}

#[cfg(test)]
#[path = "inspect_test.rs"]
mod tests;
