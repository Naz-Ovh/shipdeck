//! The one-time server setup. `server-setup.sh` remains for servers set up
//! by hand. Guided setup runs the same work from the app: packages, the app
//! folder, and password-free sudo for every command or for exactly the ones
//! shipdeck runs. The password, when one is needed, travels on stdin and is
//! never saved.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use shipdeck_protocol::message::request::{SetupAccess, SetupReport};

use crate::config::{Manager, Service};
use crate::remote::{control_commands, sudo_commands};
use crate::ssh::{Session, sh_quote};

pub const SCRIPT_NAME: &str = "server-setup.sh";

/// How long guided setup may take, package download included.
const SETUP_TIMEOUT: Duration = Duration::from_secs(600);

/// The script at the tag of this shipdeck version, so what runs as root only
/// changes with a new release.
#[must_use]
pub fn script_url() -> String {
    raw_url(&format!("scripts/{SCRIPT_NAME}"))
}

/// The setup command for one systemd app, without `--key`: for a server
/// whose user can already log in and needs its sudo rule renewed.
#[must_use]
pub fn command(user: &str, dir: &str, service: &str) -> String {
    format!(
        "curl -fsSL {} | bash -s -- --user {user} --dir {dir} --service {service}",
        script_url()
    )
}

/// A file in the shipdeck repository, at this version's tag.
#[must_use]
pub fn raw_url(file: &str) -> String {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let path = repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository);
    format!(
        "https://raw.githubusercontent.com/{path}/v{}/{file}",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
#[path = "setup_test.rs"]
mod tests;

/// Prepares a server once: packages, the project folder, and sudo access.
///
/// # Errors
/// Returns an error when the Linux is unsupported, sudo refuses the
/// password, or a setup command fails.
pub async fn guided(
    dir: &str,
    services: &[Service],
    session: &Arc<Session>,
    access: SetupAccess,
    sudo_password: Option<&str>,
) -> Result<SetupReport> {
    if services.is_empty() {
        bail!("guided setup has no service to prepare");
    }
    let script = guided_script(dir, services, access);
    let input = sudo_password
        .filter(|password| !password.is_empty())
        .map(|password| format!("{password}\n"));
    let output = tokio::time::timeout(
        SETUP_TIMEOUT,
        session.exec_input(&script, input.as_ref().map(String::as_bytes), |_, _| {}),
    )
    .await
    .context("guided setup took too long")?
    .context("running guided setup")?;
    if !output.success() {
        let detail = output
            .stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty());
        bail!(
            "guided setup failed (exit {}): {}",
            output.code,
            detail.unwrap_or("no output")
        );
    }
    let values = pairs(&output.stdout);
    let get = |key: &str| {
        values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.trim().to_string())
            .unwrap_or_default()
    };
    let root = get("root") == "yes";
    let installed = get("installed")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let notes = values
        .iter()
        .filter(|(name, _)| *name == "note")
        .map(|(_, value)| value.to_string())
        .collect();
    Ok(SetupReport {
        os: get("os"),
        user: session.user.clone(),
        dir: dir.to_string(),
        access: if root {
            "root".to_string()
        } else {
            match access {
                SetupAccess::Full => "full sudo".to_string(),
                SetupAccess::Commands => "commands".to_string(),
            }
        },
        installed,
        notes,
    })
}

/// The script guided setup runs; every value it carries is checked before.
fn guided_script(dir: &str, services: &[Service], access: SetupAccess) -> String {
    let mut script = String::from(BASE_SCRIPT);
    if has_manager(services, Manager::Systemd) {
        script.push_str("need systemctl systemd\n");
    }
    script.push_str(&PACKAGES_AND_DIR.replace("{dir}", &sh_quote(dir)));
    if has_manager(services, Manager::Systemd) {
        match access {
            SetupAccess::Full => script.push_str(FULL_SUDO_SCRIPT),
            SetupAccess::Commands => {
                script.push_str(&COMMAND_RULE.replace("{list}", &command_list(dir, services)));
            }
        }
    }
    if has_manager(services, Manager::Pm2) {
        script.push_str(PM2_SCRIPT);
    }
    script.push_str(FINAL_SCRIPT);
    script
}

fn has_manager(services: &[Service], manager: Manager) -> bool {
    services.iter().any(|service| service.manager == manager)
}

/// The sudoers entries for every systemd unit, in the script's here-doc.
fn command_list(dir: &str, services: &[Service]) -> String {
    let mut commands = Vec::new();
    for service in services {
        if service.manager == Manager::Systemd {
            commands.extend(sudo_commands(dir, &service.name));
            commands.extend(control_commands(&service.name));
        }
    }
    commands.join("\n")
}

const BASE_SCRIPT: &str = r#"set -e
umask 022
me=$(id -un)
root=no
if [ "$(id -u)" -eq 0 ]; then root=yes; fi
SUDO=''
if [ "$root" = no ]; then
  if ! sudo -n true 2>/dev/null; then
    if ! IFS= read -r password; then
      echo 'sudo needs a password: set SHIPDECK_SUDO_PASSWORD or pass --sudo-password' >&2
      exit 3
    fi
    printf '%s\n' "$password" | sudo -S -v 2>/dev/null || {
      echo 'sudo refused the password' >&2
      exit 3
    }
  fi
  SUDO='sudo -n'
fi
. /etc/os-release
case " $ID $ID_LIKE " in
  *" debian "*|*" ubuntu "*) PM=apt-get ;;
  *" fedora "*|*" rhel "*|*" centos "*) PM=dnf ;;
  *) echo "unsupported Linux: ${PRETTY_NAME:-unknown}" >&2; exit 4 ;;
esac
missing=''
need() { command -v "$1" >/dev/null 2>&1 || missing="$missing $2"; }
need zstd zstd
need tar tar
need sha256sum coreutils
need install coreutils
need visudo sudo
need curl curl
"#;

const PACKAGES_AND_DIR: &str = r#"if [ -n "$missing" ]; then
  echo "installing:$missing"
  case $PM in
    apt-get)
      $SUDO env DEBIAN_FRONTEND=noninteractive apt-get update -qq
      $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $missing
      ;;
    dnf) $SUDO dnf install -y -q $missing ;;
  esac
fi
dir={dir}
if ! mkdir -p "$dir" 2>/dev/null; then
  $SUDO install -d -m 0755 -o "$me" -g "$(id -gn)" "$dir"
fi
if [ "$(stat -c %U "$dir")" != "$me" ]; then
  $SUDO chown "$me:$(id -gn)" "$dir"
fi
echo "dir=$dir"
echo "root=$root"
"#;

const FULL_SUDO_SCRIPT: &str = r#"if [ "$root" = no ]; then
  tmp=$(mktemp)
  printf '# Written by shipdeck guided setup for %s.\n%s ALL=(ALL) NOPASSWD: ALL\n' "$me" "$me" > "$tmp"
  $SUDO visudo -cf "$tmp" >/dev/null
  $SUDO install -m 0440 -o root -g root "$tmp" "/etc/sudoers.d/shipdeck-$me"
  rm -f "$tmp"
  echo 'sudo_rule=full'
fi
"#;

const COMMAND_RULE: &str = r#"if [ "$root" = no ]; then
  rule="$me ALL=(root) NOPASSWD: "
  sep=''
  while IFS= read -r entry; do
    [ -n "$entry" ] || continue
    tool=${entry%% *}
    path=$(command -v "$tool") || { echo "missing $tool" >&2; exit 5; }
    rule="$rule$sep$path ${entry#* }"
    sep=', '
  done <<'SHIPDECK_COMMANDS'
{list}
SHIPDECK_COMMANDS
  tmp=$(mktemp)
  printf '# Written by shipdeck guided setup for %s.\n%s\n' "$me" "$rule" > "$tmp"
  $SUDO visudo -cf "$tmp" >/dev/null
  $SUDO install -m 0440 -o root -g root "$tmp" "/etc/sudoers.d/shipdeck-$me"
  rm -f "$tmp"
  echo 'sudo_rule=commands'
fi
"#;

const PM2_SCRIPT: &str = r#"if command -v pm2 >/dev/null 2>&1; then
  home=$(getent passwd "$me" | cut -d: -f6)
  if ! systemctl is-enabled "pm2-$me" >/dev/null 2>&1; then
    if ! $SUDO env PATH="$PATH" pm2 startup systemd -u "$me" --hp "$home" >/dev/null 2>&1; then
      echo 'note=PM2 could not be installed to start at boot'
    fi
  fi
else
  echo 'note=PM2 is not installed for this user'
fi
"#;

const FINAL_SCRIPT: &str = "echo \"os=${PRETTY_NAME:-$ID}\"\necho \"installed=$missing\"\n";

/// `key=value` lines from a script, in order.
fn pairs(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .collect()
}
