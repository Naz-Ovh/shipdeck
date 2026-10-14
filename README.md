# shipdeck

A terminal UI that builds an app on your own machine, ships it to a server
over SSH, and runs it there as a systemd service. Every deploy is kept as its
own release, so any of them can go live again from the Releases screen.

It exists for servers too small to build on. Whatever the language (Rust, Go,
Node.js, Python, ...), the build happens locally, and only its output, the
`.env` and any extra files travel. What the server still has to do itself,
such as installing Node dependencies, runs there as a hook inside the new
release, before it goes live.

## Run it

```sh
cargo run --release -p shipdeck -- <deploy-root> [--key <path> | --password <value>]

# or install it once
cargo install --git https://github.com/Naz-Ovh/shipdeck shipdeck
shipdeck ./scripts/deploy --key ~/.ssh/id_ed25519
shipdeck ./scripts/deploy                 # asks for the password in the TUI
```

| Flag | Login |
| --- | --- |
| `--key <path>` | Private key file. An encrypted key asks for its passphrase in the TUI. |
| `--password <value>` | Password. It lands in your shell history; in zsh with `HIST_IGNORE_SPACE`, start the command with a space. |
| neither | The host's `IdentityFile` from `~/.ssh/config` if it has one; if there is none or the server refuses it, the TUI asks for the password once per `user@host` and keeps it in memory for the run. |

`ssh.host` may be an alias from `~/.ssh/config`, so `host: prod` works like
`ssh prod`: `HostName`, `Port`, `User` and `IdentityFile` come from the
alias, and anything set in `deploy.yaml` wins. `Include` files are read,
`Match` blocks are not, and an alias that needs `ProxyJump` or `ProxyCommand`
is refused with a clear error.

The first connection to a server shows its host key fingerprint and asks
before saving it to `~/.ssh/known_hosts`. A changed key is refused.

## Deploy root

Every subfolder of the deploy root that holds a `deploy.yaml` (or
`deploy.yml`) is one deployable. Commit `deploy.yaml` and extra files; keep
`.env` out of git.

```
scripts/deploy/
├── api-prod/
│   ├── deploy.yaml          committed
│   ├── service.template     optional: replaces the built-in systemd unit
│   ├── files/               optional: bundled as they are
│   └── .env                 gitignored
└── web-prod/
```

Coming from shipdeck 0.1? It read `deploy.json`; a folder with only that file
is listed under Problems. JSON is valid YAML, so renaming it to `deploy.yaml`
is enough; then add whatever new fields you want.

### deploy.yaml

A Rust service:

```yaml
name: My API (prod)

ssh:
  host: prod                    # an address, or an alias from ~/.ssh/config
  user: deploy                  # optional when the alias has a User

build:
  cwd: ../../../my-api          # relative to this folder
  command: [cargo, build, --release, -p, my-api]
  binary: target/release/my-api

remote:
  dir: /opt/my-api
  service: my-api
  args: [-E=.env]

bundle: [.env]

health:
  url: http://127.0.0.1:8080/health
```

A Node.js app, built here, with its production dependencies installed on the
server:

```yaml
name: Web (prod)

ssh:
  host: prod

hooks:
  before: [npm test]            # this machine, before anything else
  server: [npm ci --omit=dev]   # the server, inside the new release, before it goes live
  after: [./notify.sh]          # this machine, once it is live

build:
  cwd: ../../../web
  command: npm ci && npm run build
  files: [dist, package.json, package-lock.json]

remote:
  dir: /opt/web
  service: web
  start: node dist/main.js
  requires: [npm]
  env_file: .env

bundle: [.env]
```

[`examples/`](examples) has complete, commented files for
[Rust](examples/rust/deploy.yaml), [Go](examples/go/deploy.yaml),
[Node.js](examples/node/deploy.yaml) and
[Python](examples/python/deploy.yaml); a test keeps them valid.

| Field | Default | Meaning |
| --- | --- | --- |
| `name` | | Shown in the TUI and in the unit's `Description=` |
| `ssh.host` | | An address or a `~/.ssh/config` alias |
| `ssh.user` | alias `User` | Who to log in as. Required if the alias has no `User`. |
| `ssh.port` | alias `Port`, else `22` | |
| `hooks.before` | `[]` | Shell commands run on this machine in `build.cwd`, before anything else. A failure stops the deploy before the server is touched. They may create files the bundle needs, such as `.env`. |
| `hooks.server` | `[]` | Shell commands run on the server inside the new release folder, after it is unpacked and before it goes live: `npm ci --omit=dev`, a virtualenv, migrations. A failure removes the release; the live one keeps running. |
| `hooks.after` | `[]` | Shell commands run on this machine in `build.cwd` once the new release is live. A failure is only a warning. |
| `build.cwd` | `.` | Where the build and the local hooks run, relative to the deployable folder |
| `build.command` | none | A shell line (`npm ci && npm run build`, runs with `sh -c`) or a list (`[cargo, build]`, runs as given). Leave it out when the files ship as they are. Cargo's progress bar becomes the step's progress. |
| `build.binary` | none | A compiled program, relative to `build.cwd`. It lands at the release root under its file name, and its CPU architecture and glibc floor are checked against the server. |
| `build.files` | `[]` | Build output to ship (`dist`, `package.json`), relative to `build.cwd`. Each keeps its path in the release; links inside folders stay links. At least one of `binary` and `files` is needed. |
| `build.version_from` | found | The file whose version names the release, relative to the deployable folder: `Cargo.toml` (`version.workspace = true` is followed), `package.json`, `pyproject.toml`, or any file whose first line is the version. Unset, the first of those found in `build.cwd` (with `VERSION` for the last), else the latest git tag, else `0.0.0`. |
| `remote.dir` | | The app's folder on the server, e.g. `/opt/my-app` |
| `remote.service` | | systemd service name, without `.service` |
| `remote.start` | the binary | What the service runs, from the release folder: `node dist/main.js`, `./worker --port 8080`, `.venv/bin/python -m app`. A bare program name is looked up on the server at preflight and written into the unit with its full path. |
| `remote.args` | `[]` | Arguments after the start command in `ExecStart=` |
| `remote.requires` | `[]` | Programs the server must have, such as `npm`; checked at preflight. The program `remote.start` names is checked anyway. |
| `remote.env_file` | none | A file in the release, such as `.env`, whose `KEY=value` lines become the service's environment (`EnvironmentFile=`) |
| `remote.env` | `{}` | Extra `Environment=` lines for the unit |
| `remote.run_as` | `ssh.user` | The user the service runs as. It must be able to read the release files. |
| `releases.open` | `5` | Releases kept unpacked, ready for instant rollback |
| `releases.compressed` | `15` | Older releases kept as `.tar.gz` after that |
| `bundle` | `[]` | Files and folders from the deployable folder to ship. `.env*` files always get mode `0600`. |
| `health.url` | none | If set, must answer 2xx (checked from the server with curl or wget) |
| `health.timeout_secs` | `20` | How long the new release has to become healthy |

Unknown fields are an error that names the line, so a typo never passes
silently. `build.files`, `bundle`, the binary, `release.json` and the unit
all land at the release root, so two of them may not claim the same path.

### Hooks

Each hook is one shell command (`sh -c`), run in order; its output streams
into the log. They see these variables:

| Variable | `before` | `server` | `after` |
| --- | --- | --- | --- |
| `SHIPDECK_NAME`, `SHIPDECK_HOST`, `SHIPDECK_FOLDER` (the deployable folder) | ✔ | | ✔ |
| `SHIPDECK_APP_DIR` (`remote.dir`) | ✔ | ✔ | ✔ |
| `SHIPDECK_RELEASE` (the release id) | | ✔ | ✔ |
| `SHIPDECK_SHARED_DIR` (`<dir>/shared`) | | ✔ | |

Server hooks run as the deploy user over SSH, not under systemd, so they do
not get `remote.env_file`. A migration that needs the `.env` loads it first:
`set -a && . ./.env && set +a && npm run migrate`. Each release installs its
own dependencies, so a rollback finds them already in place.

### Tools on the server

The built-in unit is hardened (`ProtectHome=true` among others), so the
service cannot see `/home`. A runtime installed per user, such as Node from
`nvm`, is therefore refused at preflight with a hint: install it system-wide
(`apt`, NodeSource, `/usr/local/bin`), or use a `service.template` without
`ProtectHome`. Hooks are not affected, since they run over SSH.

### service.template

Without one, shipdeck uses a hardened unit: `Restart=always`, read-only
system, writes allowed only to `<dir>/shared`. To change it, copy that unit
from `crates/engine/src/bundle/default.service` into `service.template` and
edit it. Placeholders:

`{{name}}` `{{tag}}` `{{id}}` `{{version}}` `{{git}}` `{{dir}}` `{{service}}`
`{{user}}` `{{exec}}` (the whole `ExecStart=` command) `{{binary}}` `{{args}}`
`{{env}}` (the `EnvironmentFile=` and `Environment=` lines)

An unknown placeholder stops the deploy at Preflight; `{{binary}}` does too
when there is no `build.binary`.

## One-time server setup

The deploy user has to exist before shipdeck can log in as it, and only root
can create it. Press `u` on a deployable and shipdeck shows the one command
to run on the server as root, with that deployable's values filled in:

```sh
curl -fsSL https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.2.0/server-setup.sh \
  | bash -s -- --user deploy --dir /opt/my-api --service my-api --key 'ssh-ed25519 AAAA... me@pc'
```

`c` copies it, and shipdeck prints it again when you quit. Not root? Put
`sudo` before `bash`. The link is pinned to the release tag of your shipdeck
version, so what runs as root only changes with a new release. Read
[`server-setup.sh`](server-setup.sh) first if you like; it is short.

The script checks every value first (the same rules as `deploy.yaml`), then:

1. creates the deploy user (no password; key login only),
2. authorizes the key: the public half of `--key`, or of the alias's
   `IdentityFile` (from its `.pub` file, or read from the private key itself),
3. creates `--dir` owned by that user,
4. installs `/etc/sudoers.d/shipdeck-<service>` after `visudo` accepts it, so
   a mistake can never break sudo on the server,
5. warns if `sshd` only lets listed users in.

Running it again changes nothing that is already in place.

The sudo rule allows exactly what shipdeck needs, nothing more:

```
deploy ALL=(root) NOPASSWD: /usr/bin/systemctl daemon-reload, /usr/bin/systemctl enable my-app, /usr/bin/systemctl restart my-app, /usr/bin/install -m 0644 /opt/my-app/current/my-app.service /etc/systemd/system/my-app.service, /usr/bin/journalctl -u my-app -n 30 --no-pager
```

Because the deploy user writes the unit file it installs, it can run anything
as the service user. Give it to nobody you would not give that power.

Preflight checks all of this on every deploy and points at `u` when
something is missing.

## What a deploy does

| # | Step | If it fails |
| --- | --- | --- |
| | **Before**: `hooks.before` on this machine (only with hooks) | Stops; nothing touched |
| 1 | **Preflight**: bundle files, architecture, glibc, free disk, sudo rights, the programs in `remote.start` and `remote.requires`, service state | Stops; nothing touched |
| 2 | **Build**: runs `build.command`, checks `build.files` are there and the binary's architecture and glibc floor against the server | Stops |
| 3 | **Pack**: binary + `build.files` + `bundle` + `release.json` + unit into one `.tar.gz` in memory | Stops |
| 4 | **Upload** over SFTP (falls back to `cat` if SFTP is off), unpack into `releases/<id>/` | Removes `<id>`; nothing changed |
| | **Prepare**: `hooks.server` inside `releases/<id>/` (only with hooks) | Removes `<id>`; nothing changed |
| 5 | **Switch**: `current` → `<id>` in one rename, install its unit, `daemon-reload` | Puts `current` and the unit back |
| 6 | **Restart** the service | Rolls back |
| 7 | **Health**: `systemctl is-active` (and `health.url`) for 3 seconds in a row; a crash loop fails at once | Rolls back, shows the last 30 journal lines |
| 8 | **Rotate**: compress releases beyond `open`, delete beyond `compressed` | Warning only |
| | **After**: `hooks.after` on this machine (only with hooks) | Warning only |

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
| `u` | show the server setup command (`c` copies it) | | |
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

### Releasing

The setup command fetches `server-setup.sh` at the tag `v<version>`, so every
release needs its tag pushed before its setup command works:

```sh
# bump [workspace.package] version in Cargo.toml, commit, then
git tag v0.2.0 && git push origin main v0.2.0
```

`setup_test.rs` fails if `server-setup.sh` and `remote::sudo_commands` ever
disagree, so the script and the preflight check cannot drift apart.
