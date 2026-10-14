#!/usr/bin/env bash
# shipdeck server setup: run once as root on a server shipdeck will deploy to.
#
#   curl -fsSL https://raw.githubusercontent.com/Naz-Ovh/shipdeck/<tag>/server-setup.sh \
#     | bash -s -- --user deploy --dir /opt/my-api --service my-api --key 'ssh-ed25519 AAAA... me@pc'
#
# Press `u` on a deployable in shipdeck to get this command with your values
# filled in. Not root? Put `sudo` before `bash`.
#
# It creates the deploy user, authorizes its key, creates the app folder
# owned by that user, and installs the sudo rule shipdeck needs (checked by
# visudo first). Running it again changes nothing that is already in place.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: server-setup.sh --user NAME --dir PATH --service NAME [--key 'PUBLIC KEY']

  --user     the user shipdeck logs in as, e.g. deploy
  --dir      the app folder, e.g. /opt/my-api
  --service  the systemd service name, without .service
  --key      an SSH public key to authorize for that user (one line)
EOF
}

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
skip() { printf '\033[2m  - %s\033[0m\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

DEPLOY_USER=""
APP_DIR=""
SERVICE=""
PUBLIC_KEY=""
while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --user|--dir|--service|--key) [ $# -ge 2 ] || die "$1 needs a value" ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
  case "$1" in
    --user)    DEPLOY_USER=$2 ;;
    --dir)     APP_DIR=$2 ;;
    --service) SERVICE=$2 ;;
    --key)     PUBLIC_KEY=$2 ;;
  esac
  shift 2
done

# Every value ends up in a sudo rule run as root, so only plain names pass;
# these are the same rules shipdeck applies to deploy.yaml.
[ -n "$DEPLOY_USER" ] && [ -n "$APP_DIR" ] && [ -n "$SERVICE" ] || { usage >&2; die "--user, --dir and --service are required"; }
[[ "$DEPLOY_USER" =~ ^[A-Za-z0-9._-]+$ ]] || die "--user may only use letters, digits, '.', '_' and '-'"
[[ "$APP_DIR" =~ ^/[A-Za-z0-9._/-]+$ ]] || die "--dir must be an absolute path of letters, digits, '.', '_', '-' and '/'"
[[ "/$APP_DIR/" != *"/../"* ]] || die "--dir must not contain '..'"
[[ "$SERVICE" =~ ^[A-Za-z0-9._@-]+$ ]] || die "--service may only use letters, digits, '.', '_', '@' and '-'"
[[ "$SERVICE" != *.service ]] || die "--service is the name without .service"
if [ -n "$PUBLIC_KEY" ]; then
  [[ "$PUBLIC_KEY" != *$'\n'* && "$PUBLIC_KEY" != *$'\r'* ]] || die "--key must be a single line"
  [[ "$PUBLIC_KEY" =~ ^(ssh-|ecdsa-|sk-)[A-Za-z0-9@.-]+\ [A-Za-z0-9+/=]+(\ .*)?$ ]] \
    || die "--key must be one public key line, e.g. 'ssh-ed25519 AAAA... me@pc'"
fi

[ "$(id -u)" -eq 0 ] || die "run this as root (put sudo before bash)"
command -v systemctl >/dev/null || die "systemd is required"
command -v sudo >/dev/null || die "sudo is required (apt install sudo, or dnf install sudo)"
command -v visudo >/dev/null || die "visudo is required (it comes with sudo)"

# sudo skips files in /etc/sudoers.d whose names contain a dot.
SUDOERS_FILE="/etc/sudoers.d/shipdeck-${SERVICE//./_}"
# Exactly what shipdeck runs with sudo; shipdeck's tests keep this list in step
# with its own.
SUDO_COMMANDS=(
  "systemctl daemon-reload"
  "systemctl enable $SERVICE"
  "systemctl restart $SERVICE"
  "install -m 0644 $APP_DIR/current/$SERVICE.service /etc/systemd/system/$SERVICE.service"
  "journalctl -u $SERVICE -n 30 --no-pager"
)

# 1. The user shipdeck logs in as.
if id "$DEPLOY_USER" >/dev/null 2>&1; then
  skip "user $DEPLOY_USER already exists"
else
  say "creating user $DEPLOY_USER"
  useradd --create-home --shell /bin/bash "$DEPLOY_USER"
fi
home=$(getent passwd "$DEPLOY_USER" | cut -d: -f6)
group=$(id -gn "$DEPLOY_USER")

# 2. The key it logs in with.
if [ -n "$PUBLIC_KEY" ]; then
  install -d -m 700 -o "$DEPLOY_USER" -g "$group" "$home/.ssh"
  keys="$home/.ssh/authorized_keys"
  touch "$keys"
  key_body=$(printf '%s\n' "$PUBLIC_KEY" | awk '{print $1" "$2}')
  if grep -qF "$key_body" "$keys"; then
    skip "key already authorized for $DEPLOY_USER"
  else
    say "authorizing the key for $DEPLOY_USER"
    printf '%s\n' "$PUBLIC_KEY" >> "$keys"
  fi
  chown "$DEPLOY_USER:$group" "$keys"
  chmod 600 "$keys"
else
  skip "no --key given; set a password with: passwd $DEPLOY_USER"
fi

# 3. The app folder, owned by the deploy user.
if [ -d "$APP_DIR" ]; then
  skip "$APP_DIR already exists"
else
  say "creating $APP_DIR"
  mkdir -p "$APP_DIR"
fi
if [ "$(stat -c %U "$APP_DIR")" != "$DEPLOY_USER" ]; then
  say "giving $APP_DIR to $DEPLOY_USER"
  chown "$DEPLOY_USER:$group" "$APP_DIR"
fi

# 4. The sudo rule: only these commands, checked by visudo before it is installed.
rule="$DEPLOY_USER ALL=(root) NOPASSWD: "
sep=""
for entry in "${SUDO_COMMANDS[@]}"; do
  tool=${entry%% *}
  path=$(command -v "$tool") || die "$tool is not installed"
  rule+="$sep$path ${entry#* }"
  sep=", "
done
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
printf '# Written by shipdeck server-setup.sh for %s.\n%s\n' "$SERVICE" "$rule" > "$tmp"
visudo -cq -f "$tmp" || die "the sudo rule did not pass visudo; nothing was installed"
if [ -f "$SUDOERS_FILE" ] && cmp -s "$tmp" "$SUDOERS_FILE"; then
  skip "sudo rule already installed"
else
  say "installing $SUDOERS_FILE"
  install -m 0440 -o root -g root "$tmp" "$SUDOERS_FILE"
fi

# 5. sshd may only let listed users in.
if command -v sshd >/dev/null && allowed=$(sshd -T 2>/dev/null | grep -i '^allowusers ') \
   && ! printf '%s\n' "$allowed" | grep -qw "$DEPLOY_USER"; then
  printf '\033[1;33mwarning:\033[0m sshd only allows: %s\n' "${allowed#* }"
  printf '         add %s to AllowUsers in /etc/ssh/sshd_config and reload sshd\n' "$DEPLOY_USER"
fi

say "done: $DEPLOY_USER can deploy $SERVICE to $APP_DIR"
