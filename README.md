# shipdeck

A terminal UI that builds a program on your own machine, ships it to a server
over SSH, and runs it there as a systemd service. Every deploy is kept as its
own release, so any of them can go live again from the Releases screen.

It exists for servers too small to compile Rust: only the finished binary, its
`.env` and any extra files travel.

## Run it

```sh
cargo run --release -p shipdeck -- <deploy-root> [--key <path> | --password <value>]

# or install it once
cargo install --path bin/shipdeck
shipdeck ./scripts/deploy --key ~/.ssh/id_ed25519
shipdeck ./scripts/deploy                 # asks for the password in the TUI
```

| Flag | Login |
| --- | --- |
| `--key <path>` | Private key file. An encrypted key asks for its passphrase in the TUI. |
| `--password <value>` | Password. It lands in your shell history; in zsh with `HIST_IGNORE_SPACE`, start the command with a space. |
| neither | The TUI asks for the password once per `user@host` and keeps it in memory for the run. |

The first connection to a server shows its host key fingerprint and asks
before saving it to `~/.ssh/known_hosts`. A changed key is refused.

## Deploy root

Every subfolder of the deploy root that holds a `deploy.json` is one
deployable. Commit `deploy.json` and extra files; keep `.env` out of git.

```
scripts/deploy/
├── server-prod/
│   ├── deploy.json          committed
│   ├── service.template     optional: replaces the built-in systemd unit
│   ├── files/               optional: bundled as they are
│   └── .env                 gitignored
└── proxy-prod/
```

### deploy.json

```json
{
  "name": "Fugitive Server (prod)",
  "ssh": { "host": "203.0.113.10", "port": 22, "user": "deploy" },
  "build": {
    "cwd": "../../../Server",
    "command": ["cargo", "build", "--release", "-p", "fugitive-server"],
    "binary": "target/release/server",
    "version_from": "../../../Server/bin/server/Cargo.toml"
  },
  "remote": {
    "dir": "/opt/fugitive-server",
    "service": "fugitive-server",
    "args": ["-E=.env"]
  },
  "releases": { "open": 5, "compressed": 15 },
  "bundle": [".env", "files/"],
  "health": { "url": "http://127.0.0.1:6520/", "timeout_secs": 20 }
}
```

| Field | Default | Meaning |
| --- | --- | --- |
| `name` | | Shown in the TUI and in the unit's `Description=` |
| `ssh.host`, `ssh.user` | | Where to log in. The login method comes from the flags. |
| `ssh.port` | `22` | |
| `build.cwd` | `.` | Where the build runs, relative to the deployable folder |
| `build.command` | | Run as given. Cargo's progress bar becomes the step's progress. |
| `build.binary` | | The built binary, relative to `build.cwd`. Its file name is its name on the server. |
| `build.version_from` | `<cwd>/Cargo.toml` | The `Cargo.toml` whose version names the release. `version.workspace = true` is followed. |
| `remote.dir` | | The app's folder on the server, e.g. `/opt/my-app` |
| `remote.service` | | systemd service name, without `.service` |
| `remote.args` | `[]` | Arguments after the binary in `ExecStart=` |
| `remote.run_as` | `ssh.user` | The user the service runs as. It must be able to read the release files. |
| `remote.env` | `{}` | Extra `Environment=` lines for the unit |
| `releases.open` | `5` | Releases kept unpacked, ready for instant rollback |
| `releases.compressed` | `15` | Older releases kept as `.tar.gz` after that |
| `bundle` | `[]` | Files and folders from the deployable folder to ship with the binary. `.env*` files always get mode `0600`. |
| `health.url` | none | If set, must answer 2xx (checked from the server with curl or wget) |
| `health.timeout_secs` | `20` | How long the new release has to become healthy |

Unknown fields are an error, so a typo never passes silently.

### service.template

Without one, shipdeck uses a hardened unit: `Restart=always`, read-only
system, writes allowed only to `<dir>/shared`. To change it, copy that unit
from `crates/engine/src/bundle/default.service` into `service.template` and
edit it. Placeholders:

`{{name}}` `{{tag}}` `{{id}}` `{{version}}` `{{git}}` `{{dir}}` `{{service}}`
`{{user}}` `{{binary}}` `{{args}}` `{{env}}`

An unknown placeholder stops the deploy at Pack.

## One-time server setup

shipdeck never changes `/etc/sudoers` or creates folders as root; preflight
checks these and prints the exact commands when something is missing.

```sh
sudo adduser deploy                                  # the user shipdeck logs in as
sudo mkdir -p /opt/my-app && sudo chown deploy: /opt/my-app
sudo visudo -f /etc/sudoers.d/shipdeck-my-app        # paste the line preflight prints
```

The sudoers line allows exactly what shipdeck needs, nothing more:

```
deploy ALL=(root) NOPASSWD: /usr/bin/systemctl daemon-reload, /usr/bin/systemctl enable my-app, /usr/bin/systemctl restart my-app, /usr/bin/install -m 0644 /opt/my-app/current/my-app.service /etc/systemd/system/my-app.service, /usr/bin/journalctl -u my-app -n 30 --no-pager
```

Because `deploy` writes the unit file it installs, `deploy` can run anything
as the service user. Give it to nobody you would not give that power.

## What a deploy does

| # | Step | If it fails |
| --- | --- | --- |
| 1 | **Preflight**: architecture, glibc, free disk, sudo rights, service state | Stops; nothing touched |
| 2 | **Build**: runs `build.command`, then checks the binary's architecture and glibc floor against the server | Stops |
| 3 | **Pack**: binary + bundle + `release.json` + unit into one `.tar.gz` in memory | Stops |
| 4 | **Upload** over SFTP (falls back to `cat` if SFTP is off), unpack into `releases/<id>/` | Removes `<id>`; nothing changed |
| 5 | **Switch**: `current` → `<id>` in one rename, install its unit, `daemon-reload` | Puts `current` and the unit back |
| 6 | **Restart** the service | Rolls back |
| 7 | **Health**: `systemctl is-active` (and `health.url`) for 3 seconds in a row; a crash loop fails at once | Rolls back, shows the last 30 journal lines |
| 8 | **Rotate**: compress releases beyond `open`, delete beyond `compressed` | Warning only |

A rollback points `current` at the previous release, reinstalls that release's
unit, restarts it and checks it is active again. The failed release is
removed.

## On the server

```
/opt/my-app/
├── current -> releases/v0.3.1-ab12cd3-20261014.153012
├── releases/        newest `open` releases, unpacked
├── backups/         next `compressed` releases, as .tar.gz
└── shared/          kept across deploys; the only folder the default unit may write to
```

A release id is `v<version>-<git sha>[-dirty]-<UTC yyyymmdd.hhmmss>`. Each
release carries `release.json` (who deployed what, from which commit) and its
own unit, whose `Description=` holds the version, so `systemctl status`
shows what is live.

## Screens

| Key | Deployables | Operation | Releases |
| --- | --- | --- | --- |
| `↑` `↓` | select | scroll the log | select |
| `⏎` | deploy (asks first) | | roll back to the selected release (asks first) |
| `r` | open releases | open releases | refresh |
| `s` | refresh live status | | |
| `G` | | follow the log again | |
| `esc` | | back | back |
| `tab`, `1` `2` `3` | switch screens | | |
| `q` | quit (asks while something runs) | | |

## Development

```
bin/shipdeck/      the TUI: flags, screens, key handling
crates/engine/     everything else: config, build, bundle, ssh, remote, deploy flows
docs/plan.md       the design and the decisions behind it
```

The engine knows nothing about the screen; it reports through
`deploy::report::Reporter`. Changes to the TUI recompile only `bin/shipdeck`.

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
SHIPDECK_DUMP=/tmp/frames cargo test -p shipdeck   # writes every screen as text
```
