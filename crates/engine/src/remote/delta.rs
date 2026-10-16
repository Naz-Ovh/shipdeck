//! A release sent as the files the live release lacks: asking the server
//! which files `current` has (hashed fresh, nothing stored is trusted),
//! and unpacking with the others copied from it, then checked against the
//! receipt before the release counts as there.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;

use super::{Server, ensure_id, pairs};
use crate::deploy::report::Reporter;
use crate::ssh::sh_quote;

/// How unpacking a release went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unpacked {
    /// `releases/<id>` holds exactly what the receipt lists.
    Verified,
    /// These files, copied from `current` or unpacked, did not match the
    /// receipt; nothing of the release is left on the server.
    Mismatch(Vec<String>),
}

impl Server {
    /// The SHA-256 of each of `paths` in `current/` that is a regular file
    /// and not a link, hashed now by the server. The paths go up as a
    /// NUL-separated list, each with `./` so none reads as an option.
    pub async fn current_hashes(
        &self,
        id: &str,
        paths: &[String],
    ) -> Result<HashMap<String, String>> {
        ensure_id(id)?;
        if paths.is_empty() {
            return Ok(HashMap::new());
        }
        let list_path = format!("{}/releases/.{id}.paths", self.dir);
        self.session
            .upload(&nul_list(paths), &list_path, |_, _| {})
            .await?;
        let out = self
            .session
            .exec(&hash_script(&self.dir, id))
            .await?
            .check("hashing the files in current")?;
        Ok(parse_hashes(&out.stdout, paths))
    }

    /// Uploads what unpacking checks and copies with, next to the bundle:
    /// the receipt, and for each mode the files to copy from `current`.
    pub async fn upload_plan(
        &self,
        id: &str,
        receipt: &[u8],
        reuse: &BTreeMap<u32, Vec<u8>>,
    ) -> Result<()> {
        ensure_id(id)?;
        let releases = format!("{}/releases", self.dir);
        self.session
            .upload(receipt, &format!("{releases}/.{id}.sha256"), |_, _| {})
            .await?;
        for (mode, list) in reuse {
            let path = format!("{releases}/.{id}.reuse.{mode:04o}");
            self.session.upload(list, &path, |_, _| {}).await?;
        }
        Ok(())
    }

    /// Unpacks the uploaded bundle into `releases/<id>/`, through a hidden
    /// folder so a half-written release never looks complete; copies the
    /// files listed for each of `modes` from `current` and gives them that
    /// mode; then checks every file against the receipt. Only a release
    /// that matches is moved into place.
    pub async fn unpack(&self, id: &str, modes: &[u32], report: &Reporter) -> Result<Unpacked> {
        ensure_id(id)?;
        let what = format!("unpack into releases/{id}");
        let out = self
            .exec_logged(&what, &unpack_script(&self.dir, id, modes), report)
            .await?;
        let values = pairs(&out.stdout);
        if values.contains(&("verified", "no")) {
            return Ok(Unpacked::Mismatch(mismatches(&values)));
        }
        out.check(&what)?;
        Ok(Unpacked::Verified)
    }
}

/// `./<path>` for each path, each ending in a NUL.
fn nul_list(paths: &[String]) -> Vec<u8> {
    let mut list = Vec::new();
    for path in paths {
        list.extend_from_slice(b"./");
        list.extend_from_slice(path.as_bytes());
        list.push(0);
    }
    list
}

/// Hashes, in `current/`, the listed paths that are regular files and not
/// links, then removes the list. A file `sha256sum` cannot read fails the
/// script, and the release is sent whole.
pub(super) fn hash_script(dir: &str, id: &str) -> String {
    format!(
        "cd {current} || exit 1
status=0
xargs -0 sh -c '
for f in \"$@\"; do
  shift
  if [ -f \"$f\" ] && [ ! -L \"$f\" ]; then set -- \"$@\" \"$f\"; fi
done
[ $# -eq 0 ] || sha256sum -- \"$@\"
' sh < {list} || status=$?
rm -f {list}
exit $status",
        current = sh_quote(&format!("{dir}/current")),
        list = sh_quote(&format!("{dir}/releases/.{id}.paths")),
    )
}

/// Reads `sha256sum` lines for the paths that were sent: 64 hex digits,
/// two characters, then `./` and one of `sent` exactly. Anything else is
/// left out, so that file is sent.
pub(super) fn parse_hashes(stdout: &str, sent: &[String]) -> HashMap<String, String> {
    let sent: HashSet<&str> = sent.iter().map(String::as_str).collect();
    stdout
        .split('\n')
        .filter_map(|line| {
            let (hash, rest) = line.split_at_checked(64)?;
            if !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return None;
            }
            let path = rest.get(2..)?.strip_prefix("./")?;
            sent.contains(path)
                .then(|| (path.to_string(), hash.to_string()))
        })
        .collect()
}

/// The unpack script. `tar -I zstd` rather than `--zstd`, which needs GNU
/// tar 1.31; tar fails when zstd does. With `modes`, the files listed for each are copied
/// from `current` with a tar pipe (mtimes kept, never hard links: the
/// release `current` points to must stay as it is), checked to be regular
/// files and given that mode. `sha256sum -c` then checks the whole release.
/// A mismatch prints `mismatch=` lines and `verified=no`, and removes the
/// release and its temporary files.
pub(super) fn unpack_script(dir: &str, id: &str, modes: &[u32]) -> String {
    let releases = format!("{dir}/releases");
    let quoted = |name: &str| sh_quote(&format!("{releases}/{name}"));
    let mut script = format!(
        "set -e
cd {releases_q}
if [ -e {id} ]; then echo 'releases/{id} already exists' >&2; exit 1; fi
rm -rf .{id}.partial
mkdir .{id}.partial
tar -I zstd -xpf .{id}.tar.zst -C .{id}.partial
rm -f .{id}.tar.zst
ok=yes
",
        releases_q = sh_quote(&releases),
    );
    for mode in modes {
        let list = quoted(&format!(".{id}.reuse.{mode:04o}"));
        script.push_str(&format!(
            "(cd {current} && tar -cf - --null --no-recursion -T {list}) | (cd .{id}.partial && tar -xpf -) || ok=no
(cd .{id}.partial && xargs -0 sh -c '
for f in \"$@\"; do
  if [ -L \"$f\" ] || [ ! -f \"$f\" ]; then echo \"mismatch=$f\"; exit 1; fi
done
chmod {mode:04o} \"$@\"
' sh < {list}) || ok=no
",
            current = sh_quote(&format!("{dir}/current")),
        ));
    }
    script.push_str(&format!(
        "(cd .{id}.partial && sha256sum -c {receipt}) > .{id}.checked || ok=no
if [ \"$ok\" = no ]; then
  sed -n 's/^\\(.*\\): FAILED.*$/mismatch=\\1/p' .{id}.checked
  rm -rf .{id}.partial .{id}.paths .{id}.sha256 .{id}.checked .{id}.reuse.*
  echo verified=no
  exit 1
fi
rm -f .{id}.paths .{id}.sha256 .{id}.checked .{id}.reuse.*
mv .{id}.partial {id}",
        receipt = quoted(&format!(".{id}.sha256")),
    ));
    script
}

/// The files `mismatch=` lines name, without `./`, each once.
fn mismatches(values: &[(&str, &str)]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (_, name) in values.iter().filter(|(key, _)| *key == "mismatch") {
        let name = name.strip_prefix("./").unwrap_or(name).to_string();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
#[path = "delta_test.rs"]
mod tests;
