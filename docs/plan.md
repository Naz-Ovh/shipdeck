# shipdeck: plan

shipdeck is a terminal UI that builds a program on your own PC, ships it to a
server over SSH, and runs it there as a systemd service. Every deploy is kept
as a separate release, so you can roll back to an earlier version at any time.

It exists because the production server is too small to compile Rust. The
build happens locally, and only the finished binary, its `.env` and any extra
files go to the server.

## Decisions

| Topic | Decision |
| --- | --- |
| Name | `shipdeck` |
| Repository | `Naz-Ovh/shipdeck` (private), added to the Fugitive workspace as the submodule `scripts/shipdeck` |
| Language and libraries | Rust: `ratatui` + `crossterm` for the TUI, `russh` + `russh-sftp` for SSH, `tar` + `flate2` for bundles, `clap` for flags, `serde_json` for config |
| Code layout | A Cargo workspace: `crates/engine` does the work, `bin/shipdeck` is the TUI, so screen changes recompile only the small crate |
| SSH session | One session per deployable, opened once and reused for every command and upload |
| SSH login | `--key <path>` or `--password <value>`; with neither flag, the TUI asks for the password when it connects |
| Configuration | One folder per deployable under a root folder passed on the command line |
| What is committed | `deploy.json` and extra files are committed; `.env` and other secrets are gitignored |
| Server location | `/opt/<app>` |
| Release layout | `releases/<id>/` folders plus a `current` symlink; systemd only uses `current/...` |
| Retention | Set per deployable: `open` releases unpacked, then `compressed` releases as `.tar.gz`. The default is 5 + 15, so 20 versions in total |
| Version visibility | Each release carries `release.json` and its own systemd unit, with the version in `Description=`, so `systemctl status` shows it |
| Failure handling | A failed health check rolls back automatically to the previous release |

## Running it

```sh
shipdeck <deploy-root> [--key <path> | --password <value>]

shipdeck ./scripts/deploy --key ~/.ssh/id_ed25519
shipdeck ./scripts/deploy                 # asks for the password in TUI as prompt
```

`--password` puts the password in your shell history. In zsh with
`HIST_IGNORE_SPACE` set, start the command with a space to keep it out.

## Deploy root on your PC

The root folder is any path you pass in. Every subfolder that has a
`deploy.json` is one deployable, and the TUI lists them all.

```
scripts/deploy/
├── server-prod/
│   ├── deploy.json          committed
│   ├── service.template     optional, committed: replaces the built-in systemd unit
│   ├── files/               optional, committed: extra files bundled as they are
│   └── .env                 gitignored
├── server-beta/
└── proxy-prod/
```

After a fresh clone, every setting is still there. You only copy each `.env`
back in.

### deploy.json

```json
{
  "name": "Fugitive Server (prod)",
  "ssh": { "host": "1.2.3.4", "port": 22, "user": "deploy" },
  "build": {
    "cwd": "../../../Server",
    "command": ["cargo", "zigbuild", "--release",
                "--target", "x86_64-unknown-linux-gnu.2.31",
                "-p", "fugitive-server"],
    "binary": "target/x86_64-unknown-linux-gnu/release/server",
    "version_from": "../../../Server/bin/server/Cargo.toml"
  },
  "remote": { "dir": "/opt/fugitive-server", "service": "fugitive-server", "args": ["-E=.env"] },
  "releases": { "open": 5, "compressed": 15 },
  "bundle": [".env", "files/"],
  "health": { "url": "http://127.0.0.1:8080/health", "timeout_secs": 20 }
}
```

| Field | Meaning |
| --- | --- |
| `ssh` | Where to connect. The login method comes from the command-line flags, never from this file |
| `build.cwd` | Folder the build runs in, relative to this deployable folder |
| `build.command` | The build command, run as given. The glibc version in the target sets the oldest server the binary runs on |
| `build.binary` | The built binary, relative to `build.cwd` |
| `build.version_from` | The `Cargo.toml` whose `version` names the release |
| `remote.dir` | The app's folder on the server |
| `remote.service` | The systemd service name |
| `remote.args` | Optional. Arguments after the binary in `ExecStart=` |
| `remote.run_as` | Optional. The user the service runs as; defaults to `ssh.user` |
| `remote.env` | Optional. Extra `Environment=` lines for the unit |
| `releases.open` | How many releases stay unpacked and ready for instant rollback |
| `releases.compressed` | How many older releases are kept as `.tar.gz` after that |
| `bundle` | Files and folders from this deployable folder to ship with the binary |
| `health` | Optional. `systemctl is-active` is always checked; `url`, if set, must answer 2xx within `timeout_secs` |

## Layout on the server

```
/opt/fugitive-server/
├── current -> releases/v0.3.1-ab12cd3-20261014.153012
├── releases/                 newest `open` releases, unpacked
│   ├── v0.3.1-ab12cd3-20261014.153012/
│   │   ├── server
│   │   ├── .env
│   │   ├── fugitive-server.service
│   │   ├── release.json
│   │   └── ...extra files
│   └── v0.3.0-9f8e7d6-20261012.091000/
├── backups/                  next `compressed` releases, as .tar.gz
│   └── v0.2.9-1a2b3c4-20261001.120000.tar.gz
└── shared/                   kept across every deploy (logs, cache)
```

`current` is a symlink: it looks and behaves like a folder, but it only
stores which release it points to. The systemd unit uses `current` and
nothing else:

```ini
[Unit]
Description=Fugitive Server v0.3.1 (ab12cd3)

[Service]
WorkingDirectory=/opt/fugitive-server/current
ExecStart=/opt/fugitive-server/current/server -E=.env
Environment=APP_RELEASE=v0.3.1-ab12cd3-20261014.153012
```

Deploying or rolling back only changes where `current` points, so the paths
systemd sees never change.

A release id is `v<cargo version>-<git short sha>-<yyyymmdd.hhmmss>` in UTC,
with `-dirty` added after the sha when there were uncommitted changes. The
timestamp (down to the second) keeps two deploys of the same commit apart,
and releases are ordered by it, never by the version text.

`release.json` records what was deployed:

```json
{
  "id": "v0.3.1-ab12cd3-20261014.153012",
  "version": "0.3.1",
  "git": "ab12cd3",
  "dirty": false,
  "built_at": "2026-10-14T15:30:12Z",
  "deployed_by": "naz-ovh"
}
```

## The deploy, step by step

| # | Step | What happens | If it fails |
| --- | --- | --- | --- |
| 1 | Preflight | Over the SSH session: check CPU architecture, the glibc version against what the binary needs, free disk space, `sudo -n` rights and service state | Stops. Nothing has been touched |
| 2 | Build | Runs `build.command`; cargo output streams into the TUI | Stops |
| 3 | Pack | Binary + `bundle` files + `release.json` + rendered unit, into one `.tar.gz` in memory | Stops |
| 4 | Upload and unpack | Uploads over SFTP and unpacks into `releases/<id>/`. The live version keeps running | Removes `<id>`. Nothing changed |
| 5 | Switch | Points `current` at `<id>` in one rename, installs the unit from `current/` into `/etc/systemd/system/`, runs `daemon-reload` and `enable`. The old process keeps serving | Points `current` back, reinstalls the previous unit, removes `<id>` |
| 6 | Restart | Restarts the service (about 1–2 seconds of downtime) | Automatic rollback |
| 7 | Health | `systemctl is-active` and, if set, the health URL must pass 3 times in a row, one second apart. A rising systemd restart count (crash loop) fails at once | Automatic rollback, and the last 30 journal lines are shown |
| 8 | Rotate | Compresses releases beyond `open` into `backups/`, deletes tarballs beyond `compressed` | Warning only. The deploy has already succeeded |

The unit is always installed from `current/`, so the sudo rule names one
fixed path and a rollback restores the unit that belongs to that release.

**Automatic rollback** points `current` back at the previous release,
reinstalls that release's unit, restarts it, checks it is active, removes the
failed release, and reports what went wrong. On a first deploy there is no
previous release, so the failed one stays in place for inspection.

**Manual rollback** from the Releases screen does the same thing for any
release you pick. If the release is compressed, it is first unpacked back
into `releases/`.

## The screens

**Deployables.** The folders found under the root. For each: its name, host,
the version live on the server and whether the service is running.

**Deploy.** The eight steps as a checklist with live status; a progress
panel with a real bar for the running step (crates compiled, bytes packed,
bytes uploaded with speed, health seconds, releases rotated) and an overall
bar; a timeline of how long each step took; and a log pane that shows cargo
output and remote command output as it happens.

**Releases.** Every unpacked and compressed release on the server, with the
current one marked. Select one to roll back to it.

## One-time server setup

shipdeck prints these steps; it never runs them for you.

1. Create a `deploy` user to log in as.
2. Create `/opt/<app>` and make `deploy` its owner.
3. Give `deploy` passwordless sudo for only the commands shipdeck needs, in
   `/etc/sudoers.d/shipdeck-<app>`: `systemctl daemon-reload`,
   `systemctl restart <service>`, `systemctl enable <service>`, installing
   `<dir>/current/<service>.service` as `/etc/systemd/system/<service>.service`,
   and reading the service's last 30 journal lines.

Every file operation inside `/opt/<app>` then runs as `deploy` without sudo.
If the sudo rule is missing, preflight fails and shows the exact line to add.

## Code layout

Organised by subject, like the Fugitive repositories, in a Cargo workspace
so the TUI recompiles on its own.

```
bin/shipdeck/src/
├── main.rs        flags, start the TUI
└── tui/           app state, the three screens, dialogs, theme
crates/engine/src/
├── config/        deploy.json schema, finding the deployables
├── build/         release name, running the build, ELF arch and glibc checks
├── bundle/        tar.gz, rendering the systemd unit
├── release.rs     release ids and release.json
├── ssh/           the russh session, login, host keys, commands, SFTP upload
├── remote/        preflight, unpack, switch, unit, health, rotate, listing releases
└── deploy/        the deploy and rollback flows, and the progress reports
```

## Milestones

All five are done.

1. **Config and discovery.** The Cargo project, the `deploy.json` schema,
   finding deployables, and the Deployables screen.
2. **SSH.** The session, preflight checks, and live version and status on the
   Deployables screen.
3. **Deploy.** The full eight-step flow with automatic rollback.
4. **Releases.** The Releases screen, rotation with compression, and manual
   rollback.
5. **Docs.** README with the one-time server setup and an example deployable.

Tested end to end against an Ubuntu 24.04 server with systemd and sshd in a
container: first deploy, five deploys with rotation (2 open + 2 compressed),
a crashing release rolled back automatically, a manual rollback to a
compressed release, and the missing-sudo preflight.

## Still open

- **Server glibc.** Run `ldd --version | head -1` on the server. If the build
  needs a newer glibc than the server has, the Build step stops and prints the
  `cargo zigbuild` target to use instead.
- **Health endpoints.** The proxy has `/healthz`. The API server has no health
  route yet, so its deployable checks only `systemctl is-active` until one is
  added.
- **GitLab backup.** `scripts/setup-remotes.sh` sets up GitLab backups for
  Client and Server only. shipdeck needs a GitLab project if it should be
  backed up there too.
- **Later, not in the first version.** A live `journalctl -f` view, deploying
  the Client's static build, and several hosts per deployable.
