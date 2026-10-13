//! The releases on the server: listing them, keeping the newest `open`
//! unpacked and the next `compressed` as tarballs, and unpacking a tarball
//! again for a rollback.

use anyhow::Result;

use super::{Server, ensure_id};
use crate::config::Retention;
use crate::deploy::report::Reporter;
use crate::release::{Manifest, ReleaseName, is_safe_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Unpacked in `releases/`, ready to switch to.
    Open,
    /// A tarball in `backups/`.
    Compressed,
}

#[derive(Debug, Clone)]
pub struct ReleaseEntry {
    pub id: String,
    /// `None` for folders shipdeck did not name; those are never touched.
    pub name: Option<ReleaseName>,
    pub kind: Kind,
    pub current: bool,
    pub size_bytes: Option<u64>,
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
    /// Open releases to turn into tarballs.
    pub compress: Vec<String>,
    /// Open releases to delete outright (too old to keep even as a tarball).
    pub drop_open: Vec<String>,
    /// Tarballs to delete.
    pub drop_backup: Vec<String>,
}

impl RotationPlan {
    pub fn is_empty(&self) -> bool {
        self.compress.is_empty() && self.drop_open.is_empty() && self.drop_backup.is_empty()
    }

    pub fn len(&self) -> usize {
        self.compress.len() + self.drop_open.len() + self.drop_backup.len()
    }
}

/// Keeps `current` plus the newest releases up to `keep.open` unpacked, then
/// the next `keep.compressed` as tarballs; everything older goes.
pub fn plan_rotation(listing: &Listing, keep: Retention) -> RotationPlan {
    let named = |kind: Kind| {
        listing
            .entries
            .iter()
            .filter(move |e| e.kind == kind && e.name.is_some())
    };

    let mut open_slots = keep.open;
    if named(Kind::Open).any(|e| e.current) {
        open_slots = open_slots.saturating_sub(1);
    }
    let mut leaving_open = Vec::new();
    for entry in named(Kind::Open).filter(|e| !e.current) {
        if open_slots > 0 {
            open_slots -= 1;
        } else {
            leaving_open.push(entry);
        }
    }

    // Tarballs after this rotation, newest first.
    let mut tarballs: Vec<(&ReleaseEntry, bool)> = named(Kind::Compressed)
        .filter(|e| !leaving_open.iter().any(|o| o.id == e.id))
        .map(|e| (e, false))
        .chain(leaving_open.iter().map(|e| (*e, true)))
        .collect();
    tarballs.sort_by(|a, b| {
        b.0.name
            .as_ref()
            .map(|n| n.stamp)
            .cmp(&a.0.name.as_ref().map(|n| n.stamp))
    });

    let mut plan = RotationPlan::default();
    for (index, (entry, from_open)) in tarballs.into_iter().enumerate() {
        let keep_it = index < keep.compressed;
        match (from_open, keep_it) {
            (true, true) => plan.compress.push(entry.id.clone()),
            (true, false) => plan.drop_open.push(entry.id.clone()),
            (false, false) => plan.drop_backup.push(entry.id.clone()),
            (false, true) => {}
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

/// Parses the tab-separated lines the listing script prints.
pub(super) fn parse_listing(text: &str) -> Listing {
    let mut listing = Listing::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["current", target] => {
                let id = target.rsplit('/').next().unwrap_or(target).trim();
                if !id.is_empty() {
                    listing.current = Some(id.to_string());
                }
            }
            [kind @ ("open" | "compressed"), id, kb, rest @ ..] if is_safe_id(id) => {
                listing.entries.push(ReleaseEntry {
                    id: id.to_string(),
                    name: ReleaseName::parse(id),
                    kind: if *kind == "open" {
                        Kind::Open
                    } else {
                        Kind::Compressed
                    },
                    current: false,
                    size_bytes: kb.trim().parse::<u64>().ok().map(|kb| kb * 1024),
                    manifest: rest
                        .first()
                        .and_then(|json| serde_json::from_str(json).ok()),
                });
            }
            _ => {}
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
        Ok(Status {
            dir_exists: get("dir") == "yes",
            current: (!current.is_empty())
                .then(|| current.rsplit('/').next().unwrap_or(&current).to_string()),
            active: get("active"),
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
for f in backups/*.tar.gz; do
  [ -f \"$f\" ] || continue
  id=$(basename \"$f\" .tar.gz)
  kb=$(du -k \"$f\" 2>/dev/null | cut -f1)
  printf 'compressed\\t%s\\t%s\\n' \"$id\" \"$kb\"
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

    /// `releases/<id>/` → `backups/<id>.tar.gz`
    pub async fn compress(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!(
            "set -e
cd {dir}
tar -czf backups/.{id}.tar.gz.part -C releases {id}
mv -f backups/.{id}.tar.gz.part backups/{id}.tar.gz
rm -rf releases/{id}",
            dir = self.q_dir()
        );
        self.run(&format!("compress {id}"), &script, report).await?;
        Ok(())
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
        let script = format!("cd {dir} && rm -f backups/{id}.tar.gz", dir = self.q_dir());
        self.run(&format!("delete backups/{id}.tar.gz"), &script, report)
            .await?;
        Ok(())
    }

    /// `backups/<id>.tar.gz` → `releases/<id>/`, so it can be switched to.
    pub async fn restore(&self, id: &str, report: &Reporter) -> Result<()> {
        ensure_id(id)?;
        let script = format!(
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
        );
        self.run(&format!("unpack backups/{id}.tar.gz"), &script, report)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "releases_test.rs"]
mod tests;
