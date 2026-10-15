//! A release's backup, `backups/<id>.tar.zst`: which of its files go in
//! (`pipeline.backup.files`), packing it on the server after setup, and
//! unpacking it again for a rollback, which then runs setup once more.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Result, anyhow, bail};

use super::{Server, ensure_id};
use crate::config::BackupFiles;
use crate::config::glob::{Rules, Verdict};
use crate::deploy::report::Reporter;
use crate::ssh::sh_quote;

/// How a release is kept in `backups/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    /// `<id>.tar.zst`: what `pipeline.backup.files` picked, made after setup.
    Backup,
    /// `<id>.tar.gz`: the whole release folder, from before shipdeck made
    /// backups. Unpacked, it is ready to switch to as it is.
    Whole,
}

impl Archive {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Backup => "tar.zst",
            Self::Whole => "tar.gz",
        }
    }
}

/// One path in a release folder on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFile {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub is_dir: bool,
}

/// What goes into a backup.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Picked {
    /// Parents before what is inside them.
    pub paths: Vec<String>,
    /// `include` patterns that matched nothing in the release.
    pub unmatched: Vec<String>,
}

/// Picks the paths of a release that go into its backup: `own` (shipdeck's
/// files) always; `deployed` when `files.deployed` says so; whatever
/// `files.include` names; minus `files.exclude`. `shared` folders are links
/// to `shared/`, never part of a release.
pub fn pick(
    listing: &[ReleaseFile],
    deployed: &BTreeSet<String>,
    own: &[String],
    shared: &[String],
    files: &BackupFiles,
) -> Result<Picked> {
    let rules = Rules::parse(&files.include, &files.exclude)
        .map_err(|why| anyhow!("pipeline.backup.files: {why}"))?;
    let mut hits = vec![0; rules.include.len()];
    let root = rules.judge(Verdict::default(), "", true, &mut hits);
    let mut sorted: Vec<&ReleaseFile> = listing.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut folders: HashMap<&str, Verdict> = HashMap::new();
    let mut paths = Vec::new();
    for file in sorted {
        let path = file.path.as_str();
        if shared
            .iter()
            .any(|folder| Path::new(path).starts_with(Path::new(folder.as_str())))
        {
            continue;
        }
        let parent = match path.rsplit_once('/') {
            Some((parent, _)) => folders.get(parent).copied().unwrap_or(root),
            None => root,
        };
        let verdict = rules.judge(parent, path, file.is_dir, &mut hits);
        if file.is_dir {
            folders.insert(path, verdict);
        }
        let wanted = (files.deployed && deployed.contains(path)) || verdict.included();
        if own.iter().any(|o| o == path) || (wanted && !verdict.excluded()) {
            paths.push(file.path.clone());
        }
    }
    let unmatched = rules
        .include
        .iter()
        .zip(&hits)
        .filter(|(pattern, hits)| !pattern.negated && **hits == 0)
        .map(|(pattern, _)| pattern.text.clone())
        .collect();
    Ok(Picked { paths, unmatched })
}

/// Reads the records the listing script prints: `d<path>` or `f<path>`,
/// each ending in a NUL, paths as `find` gives them (`./a/b`).
pub(super) fn parse_files(text: &str) -> Vec<ReleaseFile> {
    text.split('\0')
        .filter_map(|record| {
            let (kind, path) = record.split_at_checked(1)?;
            let path = path.strip_prefix("./")?;
            let is_dir = match kind {
                "d" => true,
                "f" => false,
                _ => return None,
            };
            Some(ReleaseFile {
                path: path.to_string(),
                is_dir,
            })
        })
        .collect()
}

impl Server {
    /// Every path in `releases/<id>/`, without following links.
    pub async fn release_files(&self, id: &str) -> Result<Vec<ReleaseFile>> {
        ensure_id(id)?;
        let script = format!(
            "set -e
cd {dir}/releases/{id}
find . -type d -exec printf 'd%s\\0' {{}} +
find . ! -type d -exec printf 'f%s\\0' {{}} +",
            dir = self.q_dir()
        );
        let out = self
            .session
            .exec(&script)
            .await?
            .check(&format!("listing releases/{id}"))?;
        Ok(parse_files(&out.stdout))
    }

    /// Packs `paths` of `releases/<id>/` into `backups/<id>.tar.zst`;
    /// returns its size in bytes. The list goes to tar as NUL-separated
    /// names, each with `./` so none reads as an option.
    pub async fn back_up(&self, id: &str, paths: &[String], report: &Reporter) -> Result<u64> {
        ensure_id(id)?;
        if let Some(bad) = paths
            .iter()
            .find(|p| p.contains(char::REPLACEMENT_CHARACTER))
        {
            bail!("{bad} has a name that is not UTF-8, so it cannot go into the backup");
        }
        let mut list = Vec::new();
        for path in paths {
            list.extend_from_slice(b"./");
            list.extend_from_slice(path.as_bytes());
            list.push(0);
        }
        let list_path = format!("{}/backups/.{id}.files", self.dir);
        self.session.upload(&list, &list_path, |_, _| {}).await?;
        let script = format!(
            "set -e
cd {dir}/releases/{id}
tar -I zstd -cf {part} --no-recursion --null -T {list}
rm -f {list}
mv -f {part} {done}
echo \"kb=$(du -k {done} | cut -f1)\"",
            dir = self.q_dir(),
            list = sh_quote(&list_path),
            part = sh_quote(&format!("{}/backups/.{id}.tar.zst.part", self.dir)),
            done = sh_quote(&format!("{}/backups/{id}.tar.zst", self.dir)),
        );
        let out = self
            .run(&format!("pack backups/{id}.tar.zst"), &script, report)
            .await?;
        let kb = super::pairs(&out.stdout)
            .into_iter()
            .find(|(key, _)| *key == "kb")
            .and_then(|(_, kb)| kb.trim().parse::<u64>().ok())
            .unwrap_or(0);
        Ok(kb * 1024)
    }

    /// `backups/<id>.<archive>` → `releases/<id>/`, through a hidden folder
    /// so a half-unpacked release never looks complete. A whole release's
    /// tarball goes once unpacked, as before; a backup stays.
    pub async fn restore(&self, id: &str, archive: Archive, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = match archive {
            Archive::Backup => format!(
                "set -e
cd {dir}
if [ -d releases/{id} ]; then echo 'releases/{id} is already unpacked'; exit 0; fi
rm -rf releases/.{id}.partial
mkdir releases/.{id}.partial
tar -I zstd -xpf backups/{id}.tar.zst -C releases/.{id}.partial
mv releases/.{id}.partial releases/{id}
echo 'unpacked backups/{id}.tar.zst into releases/{id}'",
                dir = self.q_dir()
            ),
            Archive::Whole => format!(
                "set -e
cd {dir}
if [ -d releases/{id} ]; then echo 'releases/{id} is already unpacked'; exit 0; fi
rm -rf releases/.{id}.partial
mkdir -p releases/.{id}.partial
tar -xzpf backups/{id}.tar.gz -C releases/.{id}.partial
mv releases/.{id}.partial/{id} releases/{id}
rmdir releases/.{id}.partial
rm -f backups/{id}.tar.gz
echo 'unpacked backups/{id}.tar.gz into releases/{id}'",
                dir = self.q_dir()
            ),
        };
        let what = format!("unpack backups/{id}.{}", archive.extension());
        self.run(&what, &script, report).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "backup_test.rs"]
mod tests;
