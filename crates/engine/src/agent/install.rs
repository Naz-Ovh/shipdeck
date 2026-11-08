//! Downloading, checking, and atomically publishing the agent under `/opt`.
//!
//! The whole install runs as root inside one install lock. It downloads into a
//! private staging folder, verifies the checksum and printed version, and
//! publishes a complete version directory in one rename. An interrupted
//! attempt leaves staging behind but never a valid version directory.

use std::time::Duration;

use anyhow::Result;

use super::binary;
use super::layout::{self, binary_path, privileged, version_dir};
use crate::ssh::{Session, sh_quote};

/// A download on a slow link plus unpacking, with a hard stop.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Installs the published release for `version`.
///
/// # Errors
///
/// Fails when root access, a download tool, the checksum, the archive, the
/// version probe, or the atomic publish fails.
pub(super) async fn install(
    session: &Session,
    prefix: &str,
    target: &str,
    label: &str,
) -> Result<String> {
    let script = install_script(&download_body(target), label);
    run(session, prefix, &script).await?;
    Ok(binary_path(label))
}

/// Installs an explicitly chosen local development agent from `source`.
///
/// The source is user-writable. Root copies it once into private staging,
/// checks the copied bytes, and runs only the root-owned copy.
///
/// # Errors
///
/// Fails when the source cannot be read, its bytes differ, or publishing
/// fails.
pub(super) async fn install_uploaded(
    session: &Session,
    prefix: &str,
    source: &str,
    digest: &str,
    label: &str,
) -> Result<String> {
    let body = upload_body(source, digest);
    let script = install_script(&body, label);
    run(session, prefix, &script).await?;
    Ok(binary_path(label))
}

async fn run(session: &Session, prefix: &str, script: &str) -> Result<()> {
    let command = privileged(prefix, script);
    session
        .exec_raw_within(&command, INSTALL_TIMEOUT)
        .await?
        .check("installing the shipdeck agent")?;
    Ok(())
}

/// The locked, atomic installer around one source-specific body.
fn install_script(body: &str, label: &str) -> String {
    let mut script = install_head(label);
    script.push_str(body);
    script.push_str(INSTALL_TAIL);
    script
}

/// The checks and lock that run before the source-specific body.
fn install_head(label: &str) -> String {
    let root = layout::AGENT_ROOT;
    let target = version_dir(label);
    format!(
        r#"set -eu
umask 022
export LC_ALL=C
[ "$(id -u)" -eq 0 ] || {{ echo 'the agent installer must run as root' >&2; exit 1; }}
for tool in mktemp chown chmod stat sha256sum awk tar find install; do
  command -v "$tool" >/dev/null 2>&1 || {{ echo "$tool is required to install the agent" >&2; exit 1; }}
done
root={root}
version={version}
target={target}
parent=$(dirname "$root")
[ ! -L "$parent" ] || {{ echo "$parent must not be a symbolic link" >&2; exit 1; }}
[ ! -L "$root" ] || {{ echo "$root must not be a symbolic link" >&2; exit 1; }}
[ "$(stat -c %U -- /opt)" = root ] || {{ echo '/opt must be owned by root' >&2; exit 1; }}
opt_mode=$(stat -c %a -- /opt)
[ $((0$opt_mode & 022)) -eq 0 ] || {{ echo '/opt must not be writable by group or other' >&2; exit 1; }}
secure=yes
dir="$target"
while [ "$dir" != / ]; do
  if [ ! -d "$dir" ] || [ -L "$dir" ] || [ "$(stat -c %U -- "$dir")" != root ]; then
    secure=no
    break
  fi
  mode=$(stat -c %a -- "$dir")
  if [ $((0$mode & 022)) -ne 0 ]; then
    secure=no
    break
  fi
  dir=$(dirname -- "$dir")
done
mkdir -p "$root"
for dir in "$parent" "$root"; do
  chown root:root "$dir"
  chmod 0755 "$dir"
done
if [ -L "$root/.install.lock" ] || {{ [ -e "$root/.install.lock" ] && [ "$(stat -c %U -- "$root/.install.lock")" != root ]; }}; then
  rm -rf -- "$root/.install.lock"
fi
if [ -L "$root/.install.lock.d" ] || {{ [ -e "$root/.install.lock.d" ] && [ "$(stat -c %U -- "$root/.install.lock.d")" != root ]; }}; then
  rm -rf -- "$root/.install.lock.d"
fi
stage=
lock_dir="$root/.install.lock.d"
holds_dir_lock=no
cleanup() {{
  [ -z "$stage" ] || rm -rf "$stage"
  if [ "$holds_dir_lock" = yes ]; then rm -rf "$lock_dir"; fi
}}
trap cleanup EXIT
if command -v flock >/dev/null 2>&1; then
  exec 9>"$root/.install.lock"
  chmod 0600 "$root/.install.lock"
  flock -x 9
else
  waited=0
  while ! mkdir "$lock_dir" 2>/dev/null; do
    holder=$(cat "$lock_dir/pid" 2>/dev/null || true)
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      rm -rf "$lock_dir"
      continue
    fi
    waited=$((waited + 1))
    [ "$waited" -lt 120 ] || {{ echo 'timed out waiting for the agent install lock' >&2; exit 1; }}
    sleep 1
  done
  holds_dir_lock=yes
  printf '%s' "$$" > "$lock_dir/pid"
fi
stage=$(mktemp -d "$root/.install.XXXXXXXX")
if [ "$secure" = yes ] && [ -f "$target/agent" ] && [ ! -L "$target/agent" ]; then
  if [ "$(stat -c %U -- "$target/agent")" = root ]; then
    mode=$(stat -c %a -- "$target/agent")
    if [ $((0$mode & 022)) -eq 0 ] && [ "$("$target/agent" --version 2>/dev/null || true)" = "shipdeck-agent $version" ]; then
      exit 0
    fi
  fi
fi
mkdir "$stage/publish"
"#,
        root = sh_quote(root),
        version = sh_quote(label),
        target = sh_quote(&target),
    )
}

/// The publish and cleanup that run after the source-specific body.
const INSTALL_TAIL: &str = r#"chown root:root "$stage/publish"
chmod 0755 "$stage/publish"
[ "$(stat -c %U -- "$stage/publish/agent")" = root ] || { echo 'the staged agent is not root-owned' >&2; exit 1; }
chmod 0755 "$stage/publish/agent"
[ "$("$stage/publish/agent" --version 2>/dev/null || true)" = "shipdeck-agent $version" ] || { echo "the agent does not report version $version" >&2; exit 1; }
rm -rf -- "$target"
mv "$stage/publish" "$target"
for old in "$root"/v*; do
  [ -e "$old" ] || continue
  [ "$old" = "$target" ] && continue
  case "$old" in
    "$root"/v*) rm -rf -- "$old" ;;
    *) echo "refusing to remove unexpected path $old" >&2; exit 1 ;;
  esac
done
[ "$("$target/agent" --version 2>/dev/null || true)" = "shipdeck-agent $version" ] || { echo 'the installed agent did not survive the publish' >&2; exit 1; }
"#;

/// The body that downloads the release archive and its checksum.
fn download_body(target: &str) -> String {
    let url = binary::asset_url(target);
    let checksum = format!("{url}.sha256");
    let url = sh_quote(&url);
    let checksum = sh_quote(&checksum);
    format!(
        r#"archive="$stage/agent.tar.gz"
sum="$stage/agent.sha256"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL --connect-timeout 10 --max-time 45 {url} -o "$archive"
  curl -fsSL --connect-timeout 10 --max-time 45 {checksum} -o "$sum"
elif command -v wget >/dev/null 2>&1; then
  wget -q --timeout=15 --tries=2 -O "$archive" {url}
  wget -q --timeout=15 --tries=2 -O "$sum" {checksum}
else
  echo 'neither curl nor wget is installed' >&2
  exit 1
fi
expected=$(awk 'NR == 1 {{ print $1 }}' "$sum")
actual=$(sha256sum "$archive" | awk '{{ print $1 }}')
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  echo 'the agent archive does not match its checksum' >&2
  exit 1
fi
bad=$(tar -tzf "$archive" | awk 'BEGIN {{ bad = 0 }} /^\// || /(^|\/)\.\.(\/|$)/ {{ bad = 1 }} END {{ print bad }}')
[ "$bad" = 0 ] || {{ echo 'the agent archive has an unsafe path' >&2; exit 1; }}
bad=$(tar -tvzf "$archive" | awk 'BEGIN {{ bad = 0 }} {{ kind = substr($1, 1, 1); if (kind != "-" && kind != "d") bad = 1 }} END {{ print bad }}')
[ "$bad" = 0 ] || {{ echo 'the agent archive holds a link or special file' >&2; exit 1; }}
mkdir "$stage/unpack"
tar -xzf "$archive" -C "$stage/unpack"
links=$(find "$stage/unpack" -type l -print | head -n 1)
[ -z "$links" ] || {{ echo 'the agent archive holds a symbolic link' >&2; exit 1; }}
found=$(find "$stage/unpack" -type f -name shipdeck-agent -print)
count=$(printf '%s\n' "$found" | awk 'NF {{ n++ }} END {{ print n + 0 }}')
[ "$count" -eq 1 ] || {{ echo 'the agent archive must hold one shipdeck-agent' >&2; exit 1; }}
install -m 0755 -o root -g root "$found" "$stage/publish/agent"
"#
    )
}

/// The body that copies a user-writable development binary into root staging.
fn upload_body(source: &str, digest: &str) -> String {
    format!(
        r#"install -m 0755 -o root -g root {source} "$stage/publish/agent"
actual=$(sha256sum "$stage/publish/agent" | awk '{{ print $1 }}')
[ "$actual" = {digest} ] || {{ echo 'the development agent checksum did not match' >&2; exit 1; }}
"#,
        source = sh_quote(source),
        digest = sh_quote(digest),
    )
}

#[cfg(test)]
#[path = "install_test.rs"]
mod tests;
