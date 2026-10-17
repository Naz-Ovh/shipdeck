//! The releases on the server: listing them, and keeping the newest `open`
//! unpacked and the next `compressed` only as their backup.

use anyhow::Result;

use super::{Server, ensure_id};
use crate::config::{Keep, Manager};
use crate::deploy::report::Reporter;
use crate::release::{Manifest, ReleaseName, is_safe_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Unpacked in `releases/`, ready to switch to.
    Open,
    /// Only its tarball in `backups/`.
    Compressed,
}

#[derive(Debug, Clone)]
pub struct ReleaseEntry {
    pub id: String,
    /// `None` for folders shipdeck did not name; those are never touched.
    pub name: Option<ReleaseName>,
    pub kind: Kind,
    pub current: bool,
    /// The folder's size when open, else the tarball's.
    pub size_bytes: Option<u64>,
    /// Whether `backups/<id>.tar.zst` holds it.
    pub backup: bool,
    pub manifest: Option<Manifest>,
}

/// Every release on the server, newest first.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub current: Option<String>,
    pub entries: Vec<ReleaseEntry>,
}

/// The live state shown next to each deployable.
#[derive(Debug, Clone)]
pub struct Status {
    pub dir_exists: bool,
    pub current: Option<String>,
    pub active: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RotationPlan {
    /// Open releases whose folder goes; their backup stays.
    pub close: Vec<String>,
    /// Open releases to delete outright (too old to keep even as a backup).
    pub drop_open: Vec<String>,
    /// Backups to delete.
    pub drop_backup: Vec<String>,
}

impl RotationPlan {
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.close.len() + self.drop_open.len() + self.drop_backup.len()
    }
}

/// Keeps `current` plus the newest releases up to `keep.open` unpacked, then
/// the next `keep.compressed` only as their backup; everything older goes.
pub fn plan_rotation(listing: &Listing, keep: Keep) -> RotationPlan {
    let named: Vec<&ReleaseEntry> = listing
        .entries
        .iter()
        .filter(|e| e.name.is_some())
        .collect();
    let mut open_slots = keep.open;
    if named.iter().any(|e| e.current) {
        open_slots = open_slots.saturating_sub(1);
    }
    let mut backup_slots = keep.compressed;
    let mut plan = RotationPlan::default();
    // Newest first, as the listing is.
    for entry in named.into_iter().filter(|e| !e.current) {
        let id = entry.id.clone();
        let open = entry.kind == Kind::Open;
        if open && open_slots > 0 {
            open_slots -= 1;
        } else if backup_slots > 0 {
            backup_slots -= 1;
            // An open release with no backup cannot stay as one.
            match (open, entry.backup) {
                (true, true) => plan.close.push(id),
                (true, false) => plan.drop_open.push(id),
                (false, _) => {}
            }
        } else {
            if open {
                plan.drop_open.push(id.clone());
            }
            if entry.backup {
                plan.drop_backup.push(id);
            }
        }
    }
    plan
}

fn by_newest(entries: &mut [ReleaseEntry]) {
    entries.sort_by(|a, b| match (&a.name, &b.name) {
        (Some(a), Some(b)) => b.stamp.cmp(&a.stamp),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.id.cmp(&b.id),
    });
}

/// Parses the tab-separated lines the listing script prints: `current`,
/// `open` for each folder, `backup` for each tarball.
pub(super) fn parse_listing(text: &str) -> Listing {
    let mut listing = Listing::default();
    let mut backups: Vec<(&str, Option<u64>)> = Vec::new();
    let kb = |text: &str| text.trim().parse::<u64>().ok().map(|kb| kb * 1024);
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["current", target] => {
                let id = target.rsplit('/').next().unwrap_or(target).trim();
                if !id.is_empty() {
                    listing.current = Some(id.to_string());
                }
            }
            ["open", id, size, rest @ ..] if is_safe_id(id) => {
                listing.entries.push(ReleaseEntry {
                    id: id.to_string(),
                    name: ReleaseName::parse(id),
                    kind: Kind::Open,
                    current: false,
                    size_bytes: kb(size),
                    backup: false,
                    manifest: rest
                        .first()
                        .and_then(|json| serde_json::from_str(json).ok()),
                });
            }
            ["backup", id, size] if is_safe_id(id) => backups.push((id, kb(size))),
            _ => {}
        }
    }
    for (id, size) in backups {
        match listing.entries.iter_mut().find(|e| e.id == id) {
            Some(entry) => entry.backup = true,
            None => listing.entries.push(ReleaseEntry {
                id: id.to_string(),
                name: ReleaseName::parse(id),
                kind: Kind::Compressed,
                current: false,
                size_bytes: size,
                backup: true,
                manifest: None,
            }),
        }
    }
    for entry in &mut listing.entries {
        entry.current = entry.kind == Kind::Open && listing.current.as_deref() == Some(&entry.id);
    }
    by_newest(&mut listing.entries);
    listing
}

impl Server {
    pub async fn status(&self) -> Result<Status> {
        let script = format!(
            "if [ -d {dir} ]; then echo dir=yes; else echo dir=no; fi
echo \"current=$(readlink {dir}/current 2>/dev/null)\"
echo \"active=$(systemctl is-active {svc} 2>&1 | head -n1)\"",
            dir = self.q_dir(),
            svc = self.q_service(),
        );
        let out = self.session.exec(&script).await?;
        let values = super::pairs(&out.stdout);
        let get = |key: &str| {
            values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default()
        };
        let current = get("current");
        let active = match self.manager {
            Manager::Systemd => get("active"),
            Manager::Pm2 => self.pm2_state().await?.0,
        };
        Ok(Status {
            dir_exists: get("dir") == "yes",
            current: (!current.is_empty())
                .then(|| current.rsplit('/').next().unwrap_or(&current).to_string()),
            active,
        })
    }

    pub async fn list(&self) -> Result<Listing> {
        let script = format!(
            "cd {dir} 2>/dev/null || exit 0
printf 'current\\t%s\\n' \"$(readlink current 2>/dev/null)\"
for d in releases/*/; do
  [ -d \"$d\" ] || continue
  id=$(basename \"$d\")
  kb=$(du -sk \"$d\" 2>/dev/null | cut -f1)
  js=$(tr -d '\\n\\t' < \"$d/release.json\" 2>/dev/null)
  printf 'open\\t%s\\t%s\\t%s\\n' \"$id\" \"$kb\" \"$js\"
done
for f in backups/*.tar.zst; do
  [ -f \"$f\" ] || continue
  id=$(basename \"$f\" .tar.zst)
  kb=$(du -k \"$f\" 2>/dev/null | cut -f1)
  printf 'backup\\t%s\\t%s\\n' \"$id\" \"$kb\"
done",
            dir = self.q_dir()
        );
        let out = self
            .session
            .exec(&script)
            .await?
            .check("listing releases")?;
        Ok(parse_listing(&out.stdout))
    }

    pub async fn drop_open(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!("cd {dir} && rm -rf releases/{id}", dir = self.q_dir());
        self.run(&format!("delete releases/{id}"), &script, report)
            .await?;
        Ok(())
    }

    pub async fn drop_backup(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!("cd {dir} && rm -f backups/{id}.tar.zst", dir = self.q_dir());
        self.run(&format!("delete the backup of {id}"), &script, report)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "releases_test.rs"]
mod tests;
