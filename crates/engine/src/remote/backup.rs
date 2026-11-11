//! A release's backup, `backups/<id>.tar.zst`: which of its files go in
//! (`pipeline.backup.files`), packing it on the server after setup, and
//! unpacking it again for a rollback, which then runs setup once more.

use anyhow::{Result, bail};

use super::{Server, ensure_id};
use crate::deploy::report::{LogKind, Reporter};
use crate::ssh::sh_quote;

/// One path in a release folder on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFile {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub is_dir: bool,
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
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe or the listing fails.
    pub async fn release_files(&self, id: &str) -> Result<Vec<ReleaseFile>> {
        ensure_id(id)?;
        if let Some(agent) = self.session.agent() {
            let files = agent.release_files(&self.dir, id).await?;
            return Ok(files
                .into_iter()
                .map(|file| ReleaseFile {
                    path: file.path,
                    is_dir: file.is_dir,
                })
                .collect());
        }
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
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe, a path is not UTF-8,
    /// or sending the list or packing fails.
    pub async fn back_up(&self, id: &str, paths: &[String], report: &Reporter) -> Result<u64> {
        ensure_id(id)?;
        if let Some(agent) = self.session.agent() {
            report.log(
                LogKind::Command,
                format!("{} › pack backups/{id}.tar.zst", self.session.label),
            );
            return agent.create_backup(&self.dir, id, paths).await;
        }
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

    /// `backups/<id>.tar.zst` → `releases/<id>/`, through a hidden folder so
    /// a half-unpacked release never looks complete. The backup stays.
    ///
    /// # Errors
    /// Returns an error when the release name is unsafe or unpacking fails.
    pub async fn restore(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        if let Some(agent) = self.session.agent() {
            report.log(
                LogKind::Command,
                format!("{} › unpack backups/{id}.tar.zst", self.session.label),
            );
            agent.restore_backup(&self.dir, id).await?;
            return Ok(());
        }
        let script = format!(
            "set -e
cd {dir}
if [ -d releases/{id} ]; then echo 'releases/{id} is already unpacked'; exit 0; fi
rm -rf releases/.{id}.partial
mkdir releases/.{id}.partial
tar -I zstd -xpf backups/{id}.tar.zst -C releases/.{id}.partial
mv releases/.{id}.partial releases/{id}
echo 'unpacked backups/{id}.tar.zst into releases/{id}'",
            dir = self.q_dir()
        );
        self.run(&format!("unpack backups/{id}.tar.zst"), &script, report)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "backup_test.rs"]
mod tests;
