# shipdeck: plan

shipdeck is a terminal UI that builds a program on your own PC, ships it to a
server over SSH, and runs it there as a systemd service. Every deploy is kept
as a separate release, so you can roll back to an earlier version at any time.

It exists because the production server is too small to compile Rust, and
grew to deploy any app built locally: Rust, Go, Node.js, Python. The build
happens on your PC, and only what `pipeline.pack` lists goes to the server.
Each step of the pipeline can run your own commands, and `deploy.yaml` says
plainly which run on your PC and which on the server; `npm ci --omit=dev`, for
instance, runs on the server in setup, inside the new release, before it goes
live.

## Decisions

| Topic | Decision |
| --- | --- |
| Name | `shipdeck` |
| Repository | `Naz-Ovh/shipdeck`, public, so servers can fetch `server-setup.sh` from it; projects add it as a git submodule |
| Language and libraries | Rust: `ratatui` + `crossterm` for the TUI, `russh` + `russh-sftp` for SSH, `tar` + `zstd` for bundles, `clap` for flags, `granit-parser` for YAML (events with line and column), `schemars` for the JSON Schema, `fast-glob` for file patterns |
| Code layout | A Cargo workspace: `crates/engine` does the work, `crates/tui` and `crates/headless` show it on a screen or as lines, and `bin/shipdeck` only parses the flags and picks one, so screen changes recompile only the small crates |
| SSH session | One session per deployable, opened once and reused for every command and upload |
| SSH login | `--key <path>` or `--password <value>`; with neither flag, the alias's `IdentityFile` from `~/.ssh/config`, then a password asked in the TUI |
| SSH host | `ssh.host` may be a `~/.ssh/config` alias (`host: prod`); its `HostName`, `Port`, `User` and `IdentityFile` apply, and `deploy.yaml` wins |
| Server setup | None needed: shipdeck deploys as `ssh.user`, whoever the owner picks (root, ubuntu, ...). The first deploy of an app creates its folder; a missing permission is named at preflight. `server-setup.sh` (`u`) can still create a user with exactly those rights for one app; a per-server setup is planned (Still open) |
| Configuration | One folder per deployable under a root folder passed on the command line, each with a `deploy.yaml` (0.1 used `deploy.json`; 0.2 a flat format; both are refused with where each key went) |
| Config check | Every `deploy.yaml` is read before the TUI starts. Any mistake, in any file, stops shipdeck with every problem printed at once, compiler style, in colour: file, line, caret, hint |
| Editor completion | The config types generate `deploy.schema.json` (draft-07), published at each release tag; examples start with the `yaml-language-server` line. The launch check walks the same schema, and a test keeps the file current |
| What is committed | `deploy.yaml` and extra files are committed; `.env` and other secrets are gitignored |
| Any app | `pipeline.pack` ships a `binary` (checked against the server's CPU and glibc), `files`, or both. `service.start` says what the service runs (`node dist/main.js`); a bare program name is found on the server at preflight and written into the unit with its full path |
| Pipeline | Eleven steps under `pipeline:`: preflight, build, pack, upload, unpack, setup, backup, switch, restart, health, done, plus failed. Each step that can run commands has `run` (and `check` where something must pass) with `local` and `server` sides; a step accepts only the sides it can run (build is local only, setup server only, upload/unpack/switch/restart none) |
| Files | `pipeline.pack.files.include` and `exclude`, each split into `project` (the source) and `deploy` (next to deploy.yaml). fast-glob/Tailwind patterns; a leading `!` leaves out (include) or keeps (exclude). `.git` never ships; a deploy file replaces a project file at the same path |
| Service environment | `service.env_file: .env` becomes `EnvironmentFile=` and is loaded for server commands too (read like systemd: no expansion); `service.env` adds single variables |
| Shared folders | `service.shared: [uploads]` lives in `shared/` and is linked into every release at unpack |
| Server location | `/opt/<app>` |
| Release layout | `releases/<id>/` folders plus a `current` symlink; systemd only uses `current/...` |
| Retention | `pipeline.backup.keep`, per deployable: `open` releases unpacked, then `compressed` releases kept only as their backup. The default is 5 + 15, so 20 versions in total |
| Backups | After setup, `pipeline.backup.files` picks what of the release goes into `backups/<id>.tar.zst`: by default the shipped files, not what setup made (node_modules, .venv), so a backup is about the size of the upload. `include` adds files setup made that it cannot make again; `exclude` leaves out. Rolling back to a backup unpacks it, links `service.shared` and runs setup again. Releases from before backups are compressed whole into `.tar.gz`, as 0.5 did |
| Version visibility | Each release carries `release.json` and its own systemd unit, with the version in `Description=`, so `systemctl status` shows it |
| Failure handling | A failed switch, restart or health check asks whether to roll back (`failed.rollback: ask`, the default) and rolls back after 30 s with no answer; `rollback: auto` never asks. `n` keeps the new release live. `failed` runs after any failure |
| Manual rollback | Unpacks and sets up a backup first, then runs health and `done`, all with `SHIPDECK_ACTION=rollback` |
| PM2 | `service.manager: pm2` instead of systemd, as the SSH user with no sudo. Each release carries `<service>.pm2.json` (`cwd` is `current`), or ships the user's `pm2.ecosystem`; restart is `pm2 startOrReload --only <app> --update-env` with the service's environment loaded, then `pm2 save`. Health reads `pm2 jlist`. PM2 newer than the major version shipdeck is tested with (6) is refused at preflight |
| Without the TUI | `deploy`, `rollback` and `releases` subcommands run the same engine flows and print what it reports as lines. Nothing waits for a person: rolling back is a flag (`--on-failure`), a new host key needs `--trust-new-host`, a password comes from `--password` or `SHIPDECK_PASSWORD`. Each `Outcome` has its own exit code, which is a contract with the scripts that call shipdeck |

## Running it

```sh
shipdeck <deploy-root> [--key <path> | --password <value>]

shipdeck ./scripts/deploy --key ~/.ssh/id_ed25519
shipdeck ./scripts/deploy                 # asks for the password in TUI as prompt
shipdeck deploy ./scripts/deploy my-api   # no TUI, for scripts and CI
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
# yaml-language-server: $schema=https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.3.0/deploy.schema.json
name: My API (prod)
project: ../../../my-api
ssh: { host: prod, user: deploy }
service:
  name: my-api
  dir: /opt/my-api
  start: ./my-api -E=.env
pipeline:
  preflight:
    run:
      local: cargo test
  build:
    run:
      local: cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31 -p my-api
  pack:
    binary: target/x86_64-unknown-linux-gnu/release/my-api
    files:
      include:
        deploy: [.env, files/]
  health:
    url: http://127.0.0.1:8080/health
```

The login method comes from the command-line flags or `~/.ssh/config`, never
from this file. The README has every field, default and variable, the step
table, and the glob syntax; `examples/` has a file for Rust, Go, Node.js and
Python.

## Layout on the server

```
/opt/my-api/
├── current -> releases/v0.3.1-ab12cd3-20261014.153012
├── releases/                 newest `keep.open` releases, unpacked
│   ├── v0.3.1-ab12cd3-20261014.153012/
│   │   ├── my-api
│   │   ├── .env
│   │   ├── my-api.service
│   │   ├── release.json
│   │   └── ...extra files
│   └── v0.3.0-9f8e7d6-20261012.091000/
├── backups/                  each release's backup, made after setup
│   ├── v0.3.1-ab12cd3-20261014.153012.tar.zst
│   ├── v0.3.0-9f8e7d6-20261012.091000.tar.zst
│   └── v0.2.9-1a2b3c4-20261001.120000.tar.zst
└── shared/                   kept across every deploy; `service.shared` folders are linked from each release
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

| # | Step | shipdeck does | Your commands | If it fails |
| --- | --- | --- | --- | --- |
| 1 | Preflight | Over the SSH session: CPU architecture, glibc, free disk space, `sudo -n` rights, the programs `service.start` and `preflight.requires` name, service state. Then the deploy-folder files | `run.server` in `service.dir`, then `run.local` | Stops. Nothing has been touched |
| 2 | Build | Names the release; afterwards checks every project pattern found something and the binary's architecture and glibc against the server | `run.local` in `project`; output streams into the TUI | Stops |
| 3 | Pack | Binary + files + `release.json` + rendered unit, into one `.tar.zst` in memory (zstd level 9, every core) | — | Stops |
| 4 | Upload | Uploads over SFTP into `releases/.<id>.tar.zst` | — | Removes `<id>`. Nothing changed |
| 5 | Unpack | Unpacks into `releases/<id>/` through a hidden folder; links `service.shared` | — | Removes `<id>`. Nothing changed |
| 6 | Setup | Nothing: the live version keeps running | `run.server` then `check.server` in `releases/<id>/` (`npm ci --omit=dev`, migrations) | Removes `<id>`. Nothing changed |
| 7 | Backup | Lists `releases/<id>/`, picks what `backup.files` names (shipdeck's own files always, `service.shared` never), and packs it on the server into `backups/<id>.tar.zst` | — | Removes `<id>` and its backup. Nothing changed |
| 8 | Switch | Points `current` at `<id>` in one rename, installs the unit from `current/`, `daemon-reload`, `enable`. The old process keeps serving | — | Asks, then rolls back |
| 9 | Restart | Restarts the service (about 1–2 seconds of downtime) | — | Asks, then rolls back |
| 10 | Health | `systemctl is-active` and, if set, the URL must pass 3 times in a row, one second apart; a rising restart count fails at once | `check.server` in `current/`, then `check.local`, retried until `timeout_secs` | Asks, then rolls back; the last 30 journal lines are shown |
| 11 | Done | Deletes the folders of releases beyond `keep.open` (their backup stays), and folder and backup beyond `keep.compressed` | `run.server` in `current/`, then `run.local` | Warning only. The release stays live |
| ✱ | Failed | Runs when any step failed, after the rollback | `run.local`, with `SHIPDECK_STEP` naming the failed step | Logged |

The unit is always installed from `current/`, so the sudo rule names one
fixed path and a rollback restores the unit that belongs to that release.

**Rollback after a failure** asks first with `failed.rollback: ask`: a dialog shows
both releases and a countdown, and with no answer in 30 seconds shipdeck
rolls back. Rolling back points `current` at the previous release, reinstalls
that release's unit, restarts it, checks it is active, and removes the failed
release. `n` keeps the new release live as it is. On a first deploy there is
no previous release, so the failed one stays in place for inspection.

**Manual rollback** from the Releases screen does the same for any release
you pick, then runs health and `done` with `SHIPDECK_ACTION=rollback`. A
release kept only as a backup is first unpacked back into `releases/`, gets
its `service.shared` links and runs setup again; the backup stays.

## The screens

**Deployables.** The folders found under the root. For each: its name, host,
the version live on the server and whether the service is running.

**Deploy.** The steps as a checklist with live status (eleven, plus Failed
when the deployable has `failed` commands), scrolled to keep the running one in view; a progress
panel with a real bar for the running step (crates compiled, bytes packed,
bytes uploaded with speed, health seconds, releases rotated) and an overall
bar; a timeline of how long each step took; and a log pane that shows cargo
output and remote command output as it happens. A failure after the switch
opens the rollback question on top, with its countdown.

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
└── main.rs        flags, login, the config check, then the TUI or a command
crates/tui/src/    app state, the three screens, dialogs, theme
crates/headless/src/
└── lib.rs         deploy, rollback and releases without the TUI: lines, answers, exit codes
crates/engine/src/
├── config/        the deploy.yaml types (= the JSON Schema), the YAML tree with line numbers,
│                  the schema walk and value rules, the problem report, globs, discovery
├── build/         release name, running local commands, ELF arch and glibc checks
├── bundle/        picking the files, tar.zst, rendering the systemd unit
├── release.rs     release ids and release.json
├── ssh/           the russh session, ~/.ssh/config aliases, login, host keys, commands, SFTP upload
├── remote/        preflight, unpack, shared links, server commands, switch, unit, health,
│                  backups (picking the files, packing, unpacking), rotate
├── setup.rs       the server setup command (the script is server-setup.sh at the root)
└── deploy/        the deploy and rollback flows, and the progress reports
```

## Milestones

All seven are done.

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
7. **The pipeline (0.3).** `pipeline:` with local and server commands per
   step, `pack.files` with globs, `service.shared`, the rollback question,
   `done` and `failed`, every config problem reported at launch, and
   `deploy.schema.json` for editors.

Tested end to end against an Ubuntu 24.04 server with systemd and sshd in a
container: first deploy, five deploys with rotation (2 open + 2 compressed),
a crashing release rolled back automatically, a manual rollback to a
compressed release, and the missing-sudo preflight. For 0.2: a Go binary with
`build.files` and local hooks, a Node.js app whose `.env` a before hook writes
and whose `npm ci --omit=dev` runs on the server, and a failing server hook
that leaves the live release untouched. For 0.3: a Node.js app through every
step (env file and `service.env` seen by server commands, a shared folder, a
`.map` excluded, health checks on both sides, `done` on both sides); a health
check failing on purpose, answered with `y`, left unanswered for 30 s, and
answered with `n`; a manual rollback that runs `done` with
`SHIPDECK_ACTION=rollback`; a failing setup that removes the release; and a Go
worker shipping `.` minus `!cmd`, `dist` and `go.mod`, with `.git` left out.

## Still open

- **Server setup, once per server (later).** Today `server-setup.sh` sets up
  a user for one app. Planned: `curl … | bash` with no arguments, once per
  server. It asks which user shipdeck logs in as, and whether that user logs
  in with a pasted public key or a password (or keeps what it has). It
  installs `shipdeck-root`, a root-owned command the user may run with sudo,
  which creates app folders and installs, restarts and reads the journal of
  units shipdeck installed itself; a registry of those units stops a typo in
  `service.name` from replacing sshd or nginx. Each app's folder and service
  still come from its first deploy, so dozens of apps need no further setup.

- **Server glibc.** Run `ldd --version | head -1` on the server. If the build
  needs a newer glibc than the server has, the Build step stops and prints the
  `cargo zigbuild` target to use instead.
- **Later, not in the first version.** A live `journalctl -f` view, deploying
  static sites, and several hosts per deployable.
