# shipdeck: plan

shipdeck is a terminal UI that builds a program on your own PC, ships it to a
server over SSH, and runs it there as a systemd service. Every deploy is kept
as a separate release, so you can roll back to an earlier version at any time.

It exists because the production server is too small to compile Rust, and
grew to deploy any app built locally: Rust, Go, Node.js, Python. The build
happens on your PC, and only its output, the `.env` and any extra files go to
the server. What must still happen there, such as `npm ci --omit=dev`, runs
as a hook inside the new release before it goes live.

## Decisions

| Topic | Decision |
| --- | --- |
| Name | `shipdeck` |
| Repository | `Naz-Ovh/shipdeck`, public, so servers can fetch `server-setup.sh` from it; projects add it as a git submodule |
| Language and libraries | Rust: `ratatui` + `crossterm` for the TUI, `russh` + `russh-sftp` for SSH, `tar` + `flate2` for bundles, `clap` for flags, `serde-saphyr` for the YAML config (pure Rust, maintained, errors name the line; `serde_yaml` is deprecated) |
| Code layout | A Cargo workspace: `crates/engine` does the work, `bin/shipdeck` is the TUI, so screen changes recompile only the small crate |
| SSH session | One session per deployable, opened once and reused for every command and upload |
| SSH login | `--key <path>` or `--password <value>`; with neither flag, the alias's `IdentityFile` from `~/.ssh/config`, then a password asked in the TUI |
| SSH host | `ssh.host` may be a `~/.ssh/config` alias (`host: prod`); its `HostName`, `Port`, `User` and `IdentityFile` apply, and `deploy.yaml` wins |
| Server setup | `server-setup.sh` in the repo, fetched with `curl ... \| bash -s -- <values>` and run once as root; pinned to the release tag. `u` shows the command with the deployable's values |
| Configuration | One folder per deployable under a root folder passed on the command line, each with a `deploy.yaml` (0.1 used `deploy.json`; JSON is YAML, so a rename carries it over) |
| What is committed | `deploy.yaml` and extra files are committed; `.env` and other secrets are gitignored |
| Any app | The build ships a `binary` (checked against the server's CPU and glibc), `files` (build output such as `dist`), or both. `remote.start` says what the service runs (`node dist/main.js`); a bare program name is found on the server at preflight and written into the unit with its full path |
| Hooks | `hooks.before` (this machine, first of all), `hooks.server` (the server, inside the new release, before it goes live) and `hooks.after` (this machine, once live). Each hook step appears only when it has commands |
| Service environment | `remote.env_file: .env` becomes `EnvironmentFile=`; `remote.env` adds single variables |
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
`deploy.yaml` is one deployable, and the TUI lists them all.

```
scripts/deploy/
├── server-prod/
│   ├── deploy.yaml          committed
│   ├── service.template     optional, committed: replaces the built-in systemd unit
│   ├── files/               optional, committed: extra files bundled as they are
│   └── .env                 gitignored
├── server-beta/
└── proxy-prod/
```

After a fresh clone, every setting is still there. You only copy each `.env`
back in.

### deploy.yaml

```yaml
name: My API (prod)
ssh: { host: prod, user: deploy }
hooks:
  before: [cargo test]
build:
  cwd: ../../../my-api
  command: [cargo, zigbuild, --release, --target, x86_64-unknown-linux-gnu.2.31, -p, my-api]
  binary: target/x86_64-unknown-linux-gnu/release/my-api
remote: { dir: /opt/my-api, service: my-api, args: [-E=.env] }
releases: { open: 5, compressed: 15 }
bundle: [.env, files/]
health: { url: "http://127.0.0.1:8080/health", timeout_secs: 20 }
```

| Field | Meaning |
| --- | --- |
| `ssh` | Where to connect. The login method comes from the command-line flags or `~/.ssh/config`, never from this file |
| `hooks` | Optional. `before`, `server` and `after` shell commands; see the deploy steps below |
| `build.cwd` | Folder the build and local hooks run in, relative to this deployable folder |
| `build.command` | Optional. A shell line or an argument list. For a compiled binary, the glibc in the target sets the oldest server it runs on |
| `build.binary` | Optional. A compiled program, relative to `build.cwd` |
| `build.files` | Optional. Build output to ship, relative to `build.cwd` (at least one of `binary` and `files`) |
| `build.version_from` | Optional. The file whose version names the release; found in `build.cwd` (Cargo.toml, package.json, pyproject.toml, VERSION), else the latest git tag |
| `remote.dir` | The app's folder on the server |
| `remote.service` | The systemd service name |
| `remote.start` | Optional. What the service runs, from the release; defaults to the binary |
| `remote.args` | Optional. Arguments after the start command in `ExecStart=` |
| `remote.requires` | Optional. Programs preflight must find on the server |
| `remote.env_file` | Optional. A file in the release loaded as the service's environment |
| `remote.run_as` | Optional. The user the service runs as; defaults to `ssh.user` |
| `remote.env` | Optional. Extra `Environment=` lines for the unit |
| `releases.open` | How many releases stay unpacked and ready for instant rollback |
| `releases.compressed` | How many older releases are kept as `.tar.gz` after that |
| `bundle` | Files and folders from this deployable folder to ship |
| `health` | Optional. `systemctl is-active` is always checked; `url`, if set, must answer 2xx within `timeout_secs` |

The README has every default, and `examples/` has a commented file for Rust,
Go, Node.js and Python.

## Layout on the server

```
/opt/my-api/
├── current -> releases/v0.3.1-ab12cd3-20261014.153012
├── releases/                 newest `open` releases, unpacked
│   ├── v0.3.1-ab12cd3-20261014.153012/
│   │   ├── my-api
│   │   ├── .env
│   │   ├── my-api.service
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
Description=My API (prod) v0.3.1 (ab12cd3)

[Service]
WorkingDirectory=/opt/my-api/current
ExecStart=/opt/my-api/current/my-api -E=.env
Environment=APP_RELEASE=v0.3.1-ab12cd3-20261014.153012
```

Deploying or rolling back only changes where `current` points, so the paths
systemd sees never change.

A release id is `v<version>-<git short sha>-<yyyymmdd.hhmmss>` in UTC,
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
  "deployed_by": "me"
}
```

## The deploy, step by step

| # | Step | What happens | If it fails |
| --- | --- | --- | --- |
| | Before | Only with `hooks.before`: runs them on this machine in `build.cwd`. They may create files the bundle needs | Stops. Nothing has been touched |
| 1 | Preflight | Checks the bundle files, then over the SSH session: CPU architecture, glibc, free disk space, `sudo -n` rights, the programs `remote.start` and `remote.requires` name, and service state | Stops. Nothing has been touched |
| 2 | Build | Runs `build.command`; its output streams into the TUI. Then checks `build.files` exist and the binary's architecture and glibc against the server | Stops |
| 3 | Pack | Binary + `build.files` + `bundle` + `release.json` + rendered unit, into one `.tar.gz` in memory | Stops |
| 4 | Upload and unpack | Uploads over SFTP and unpacks into `releases/<id>/`. The live version keeps running | Removes `<id>`. Nothing changed |
| | Prepare | Only with `hooks.server`: runs them on the server inside `releases/<id>/` (`npm ci --omit=dev`, migrations). The live version keeps running | Removes `<id>`. Nothing changed |
| 5 | Switch | Points `current` at `<id>` in one rename, installs the unit from `current/` into `/etc/systemd/system/`, runs `daemon-reload` and `enable`. The old process keeps serving | Points `current` back, reinstalls the previous unit, removes `<id>` |
| 6 | Restart | Restarts the service (about 1–2 seconds of downtime) | Automatic rollback |
| 7 | Health | `systemctl is-active` and, if set, the health URL must pass 3 times in a row, one second apart. A rising systemd restart count (crash loop) fails at once | Automatic rollback, and the last 30 journal lines are shown |
| 8 | Rotate | Compresses releases beyond `open` into `backups/`, deletes tarballs beyond `compressed` | Warning only. The deploy has already succeeded |
| | After | Only with `hooks.after`: runs them on this machine | Warning only. The release stays live |

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

**Deploy.** The steps as a checklist with live status (eight, plus one for
each kind of hook the deployable has), scrolled to keep the running one in view; a progress
panel with a real bar for the running step (crates compiled, bytes packed,
bytes uploaded with speed, health seconds, releases rotated) and an overall
bar; a timeline of how long each step took; and a log pane that shows cargo
output and remote command output as it happens.

**Releases.** Every unpacked and compressed release on the server, with the
current one marked. Select one to roll back to it.

## One-time server setup

Only root can create the deploy user, so this runs once as root.
`server-setup.sh` lives at the root of this repository. Press `u` on a
deployable and shipdeck shows the command to run on the server:

```sh
curl -fsSL https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v<version>/server-setup.sh \
  | bash -s -- --user deploy --dir /opt/<app> --service <service> --key '<public key>'
```

The link is pinned to the release tag of the shipdeck build, so what runs as
root changes only with a release. The script validates every value with the
same rules as `deploy.yaml`, then:

1. Creates the deploy user, with key login only.
2. Authorizes the key shipdeck logs in with (its public half).
3. Creates `/opt/<app>` and makes the deploy user its owner.
4. Installs `/etc/sudoers.d/shipdeck-<service>` after `visudo` accepts it,
   allowing only: `systemctl daemon-reload`, `systemctl enable <service>`,
   `systemctl restart <service>`, installing
   `<dir>/current/<service>.service` as `/etc/systemd/system/<service>.service`,
   and reading the service's last 30 journal lines. Dots in the service name
   become `_` in the file name, because sudo skips files whose names contain one.

A test fails if the script's sudo commands and shipdeck's own list differ.
Running it again changes nothing already in place. Every file operation
inside `/opt/<app>` then runs as the deploy user without sudo. Preflight
checks all of this and shows the setup command when something is missing.

## Code layout

Organised by subject, in a Cargo workspace
so the TUI recompiles on its own.

```
bin/shipdeck/src/
├── main.rs        flags, start the TUI
└── tui/           app state, the three screens, dialogs, theme
crates/engine/src/
├── config/        deploy.yaml schema, finding the deployables
├── build/         release name, running the build and local hooks, ELF arch and glibc checks
├── bundle/        tar.gz, rendering the systemd unit
├── release.rs     release ids and release.json
├── ssh/           the russh session, ~/.ssh/config aliases, login, host keys, commands, SFTP upload
├── remote/        preflight, unpack, switch, unit, health, rotate, listing releases
├── setup.rs       the server setup command (the script is server-setup.sh at the root)
└── deploy/        the deploy and rollback flows, and the progress reports
```

## Milestones

All five are done.

1. **Config and discovery.** The Cargo project, the config schema,
   finding deployables, and the Deployables screen.
2. **SSH.** The session, preflight checks, and live version and status on the
   Deployables screen.
3. **Deploy.** The full eight-step flow with automatic rollback.
4. **Releases.** The Releases screen, rotation with compression, and manual
   rollback.
5. **Docs.** README with the one-time server setup and an example deployable.
6. **Any app (0.2).** `deploy.yaml`, hooks, `build.files`, `remote.start`,
   `remote.requires`, `remote.env_file`, and versions from package.json,
   pyproject.toml, VERSION or a git tag.

Tested end to end against an Ubuntu 24.04 server with systemd and sshd in a
container: first deploy, five deploys with rotation (2 open + 2 compressed),
a crashing release rolled back automatically, a manual rollback to a
compressed release, and the missing-sudo preflight. For 0.2: a Go binary with
`build.files` and local hooks, a Node.js app whose `.env` a before hook writes
and whose `npm ci --omit=dev` runs on the server, and a failing server hook
that leaves the live release untouched.

## Still open

- **Server glibc.** Run `ldd --version | head -1` on the server. If the build
  needs a newer glibc than the server has, the Build step stops and prints the
  `cargo zigbuild` target to use instead.
- **Later, not in the first version.** A live `journalctl -f` view, deploying
  static sites, and several hosts per deployable.
