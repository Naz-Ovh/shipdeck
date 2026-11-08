# Deployment configuration

Detailed reference. Start with the [overview](../../README.md).

## Run it

```sh
cargo run --release -p shipdeck -- <deploy-root> [--key <path> | --password <value>]

shipdeck ./scripts/deploy --key ~/.ssh/id_ed25519
shipdeck ./scripts/deploy                 # asks for the password in the TUI
shipdeck --schema > deploy.schema.json    # the JSON Schema, for offline editors
```

| Flag | Login |
| --- | --- |
| `--key <path>` | Private key file. An encrypted key asks for its passphrase in the TUI, or takes it from `SHIPDECK_KEY_PASSPHRASE`. |
| `--key-text <text>` | The private key itself, such as a CI secret. It shows in `ps`; `SHIPDECK_SSH_KEY` does the same without that. Store the key file's text exactly as it is. |
| `--no-shell-env` | Server commands run without the login shell's startup files, on every server (see [Tools on the server](#tools-on-the-server)). |
| `--gitshell-path <path>` | Windows only: the `sh.exe` that runs local commands, instead of Git for Windows' own. `SHIPDECK_GITSHELL_PATH` does the same. |
| `--password <value>` | Password. It lands in your shell history; in zsh with `HIST_IGNORE_SPACE`, start the command with a space. `SHIPDECK_PASSWORD` does the same without the history. |
| neither | The host's `IdentityFile` from `~/.ssh/config` if it has one; if there is none or the server refuses it, the TUI asks for the password once per `user@host` and keeps it in memory for the run. |

### The manual

The whole manual is built into the binary, for you or a coding agent:

```sh
shipdeck man index                                   # every topic as `key: brief`
shipdeck man configs                                 # every shipdeck.yaml property, as a YAML tree
shipdeck man details agents step.health service.env  # topics and properties in full
```

`details` takes several keys, apart or comma-separated, and prints each in its
own `<topic key="...">` or `<config key="...">` block, in the order asked.

### Start a deployable

`shipdeck init` creates a deployable's folder under the deploy root with a
`shipdeck.yaml` from a preset, an empty `.env` and a `.gitignore` that keeps
`.env` out of git. In a terminal it asks for whatever the flags leave out;
with no terminal (a script, CI) it asks nothing, and NAME, `--preset` and
`--host` are required.

```sh
shipdeck init ./scripts/deploy                      # asks for everything
shipdeck init ./scripts/deploy my-api --preset rust --manager systemd --host prod
```

| Flag | Default |
| --- | --- |
| `NAME` | asked: the folder's name, also `name` and `service.name` |
| `--preset` | asked: `nodejs`, `golang`, `java`, `kotlin`, `rust`, `dart`, `python` or `bun` |
| `--manager` | asked, else `systemd`; or `pm2` |
| `--host` | asked: an address or a `~/.ssh/config` alias |
| `--user` | asked, offering the alias's `User`; else the alias's `User` |
| `--project` | asked, else the current folder; written relative to the new folder |

`dir` is `/opt/<NAME>`. What `init` writes passes the same checks as
a launch, and it never replaces an existing `shipdeck.yaml`. Each preset is a
start; check its build command and paths against your project:

| Preset | Build here | Ships | Server needs |
| --- | --- | --- | --- |
| `nodejs` | `npm ci`, `npm run build` | `dist`, `package.json`, `package-lock.json` | npm (`npm ci --omit=dev` in setup) |
| `golang` | `CGO_ENABLED=0 GOOS=linux go build` | the binary | nothing |
| `rust` | `cargo build --release` | the binary | nothing |
| `dart` | `dart compile exe bin/server.dart` | the executable | nothing |
| `bun` | `bun build --compile --target=bun-linux-x64` | the executable | nothing |
| `java`, `kotlin` | `./gradlew installDist` ([application plugin](https://docs.gradle.org/current/userguide/application_plugin.html)) | `build/install/<NAME>` | java |
| `python` | nothing | the source | python3 (a virtualenv from `requirements.txt` in setup) |

### Without the TUI

For scripts and CI, these commands do the same work with no screen. They
print progress to stderr as plain lines and never wait for an answer.

```sh
shipdeck deploy      ./scripts/deploy my-api [my-web ...] [--on-failure rollback|keep] [--allow-unchanged]
shipdeck rollback    ./scripts/deploy my-api --to v0.3.0-9f8e7d6-20261012.091000
shipdeck releases    ./scripts/deploy my-api     # the table on stdout, newest first
shipdeck server-info ./scripts/deploy my-api     # Linux, CPU, memory, disk, tools
shipdeck work        ./scripts/deploy my-api     # interrupted files and uncertain activation
shipdeck events      ./scripts/deploy my-api     # numbered events, oldest first
shipdeck setup       ./scripts/deploy my-api     # guided one-time server setup
```

A deployable is named by its folder or its `name`. `deploy` takes several and
runs them one after another; the first that does not succeed stops the rest.
Every `shipdeck.yaml` under the root is still checked first.

In CI, keep the key in a secret:

```sh
SHIPDECK_SSH_KEY="$DEPLOY_KEY" shipdeck deploy ./scripts/deploy my-api --trust-new-host
```

What the TUI would ask comes from flags instead:

| The TUI asks | Without the TUI |
| --- | --- |
| Deploy this? | Running the command is the yes. |
| Roll back after a failed switch, restart or health check? | `--on-failure rollback` (the default) rolls back at once; `--on-failure keep` keeps the failed release live. Either overrides `pipeline.failed.rollback`. |
| Nothing changed since the live release: deploy anyway? | No: the deploy stops with exit code `6` and touches nothing. `--allow-unchanged` deploys it anyway, as a duplicate. |
| Trust a server seen for the first time? | No, unless `--trust-new-host`. A changed host key is always refused. |
| A password or key passphrase? | Never asked: use `--key` or `SHIPDECK_SSH_KEY` (with `SHIPDECK_KEY_PASSPHRASE` for an encrypted key), `--password` or `SHIPDECK_PASSWORD`. |

The exit code says how it ended:

| Code | Meaning |
| --- | --- |
| `0` | The release is live. |
| `1` | It failed before the live release changed, or shipdeck could not start (a config problem, an unknown name). |
| `2` | Wrong usage (from the flag parser). |
| `3` | The new release failed and the previous one is live again. |
| `4` | The new release failed and stays live (`--on-failure keep`). |
| `5` | The new release failed and putting the old one back failed too: look at the server now. |
| `6` | Nothing changed since the live release, so nothing was deployed (`--allow-unchanged` deploys anyway). |
| `130` | Ctrl-C twice. The first one only warns, since stopping can leave the server between two releases. |

`ssh.host` may be an alias from `~/.ssh/config`, so `host: prod` works like
`ssh prod`: `HostName`, `Port`, `User` and `IdentityFile` come from the
alias, and anything set in `shipdeck.yaml` wins. `Include` files are read,
`Match` blocks are not, and an alias that needs `ProxyJump` or `ProxyCommand`
is refused with a clear error.

The first connection to a server shows its host key fingerprint and asks
before saving it to `~/.ssh/known_hosts`. A changed key is refused.

## Deploy root

Every subfolder of the deploy root that holds a `shipdeck.yaml` (or
`shipdeck.yml`) is one deployable. Commit `shipdeck.yaml` and extra files; keep
`.env` out of git.

```
scripts/deploy/
├── api-prod/
│   ├── shipdeck.yaml          committed
│   ├── service.template     optional: replaces the built-in systemd unit
│   ├── config.yaml          optional: shipped with pipeline.pack.files.include.deploy
│   └── .env                 gitignored
└── web-prod/
```

shipdeck reads every `shipdeck.yaml` before it starts. If any of them has a
mistake, it does not start, and prints every problem in every file at once
(see [Mistakes](#mistakes-are-caught-before-anything-runs)).

## shipdeck.yaml

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.7.1/docs/schema/deploy.schema.json
name: Web (prod)
project: ../../../web                       # your source; local commands run here
dir: /opt/web                               # current/ releases/ backups/ shared/
ssh:
  host: prod                                # address or ~/.ssh/config alias
  user: deploy
service:
  name: web                                 # web.service
  start: node dist/main.js                  # default: ./<binary>
  env_file: .env                            # also loaded for server commands
  env: { NODE_ENV: production }
  shared: [uploads]                         # kept across releases
pipeline:
  preflight:
    requires: [node, npm]
    run:
      server: pg_isready -q
      local: npm test
  build:
    version_from: package.json
    run:
      local: [npm ci, npm run build]
  pack:
    files:
      include:
        project: [dist, package.json, package-lock.json]
        deploy: [.env]                      # next to this shipdeck.yaml
      exclude:
        project: ["**/*.map"]
  # upload, unpack                          shipdeck
  setup:                                    # in releases/<id>, old release still live
    run:
      server:
        - npm ci --omit=dev
        - npm run migrate
    check:
      server: node --check dist/main.js     # fails → release removed
  backup:                                   # what stays once the folder goes
    files:
      include: [data/keys]                  # besides the shipped files
    keep: { open: 5, compressed: 15 }
  # switch, restart                         shipdeck
  health:
    url: http://127.0.0.1:3000/health       # fetched on the server
    check:
      server: ./bin/smoke-test
      local: curl -fsS https://web.example.com/health
    timeout_secs: 20
  done:                                     # new release is live
    run:
      server: ./bin/warm-cache
      local: ./notify.sh "web $SHIPDECK_VERSION is live"
  failed:
    rollback: ask                           # default; or auto
    run:
      local: ./notify.sh "failed at $SHIPDECK_STEP"
```

[`examples/`](../../examples) has complete files for [Rust](../../examples/rust/shipdeck.yaml),
[Go](../../examples/go/shipdeck.yaml), [Node.js](../../examples/node/shipdeck.yaml) and
[Python](../../examples/python/shipdeck.yaml), plus a [monorepo](../../examples/monorepo)
with systemd and PM2 apps in one release; a test keeps them valid.
See the [monorepo guide](docs/feature/monorepo-deploy.md) for shared releases,
app commands, backups and rollback.

### The steps

| # | Step | shipdeck does | your commands |
| --- | --- | --- | --- |
| 1 | preflight | checks your PC and the server | `run.server` → `run.local` |
| 2 | build | names the release | `run.local` |
| 3 | pack | hashes every file, asks the live release which it has, packs the rest | — |
| 4 | upload | sends the archive and its receipt to the server | — |
| 5 | unpack | unpacks it into `releases/<id>`, copies the unchanged files from `current`, checks every file, links `service.shared` | — |
| 6 | setup | nothing: the old release is still live | `run.server` → `check.server` |
| 7 | backup | packs `backup.files` into `backups/<id>.tar.zst` | — |
| 8 | switch | points `current` at `releases/<id>`, installs its unit | — |
| 9 | restart | restarts the service | — |
| 10 | health | waits until the service is active and `url` answers | `check.server` → `check.local` |
| 11 | done | tidies old releases (`backup.keep`) | `run.server` → `run.local` |
| ✱ | failed | a step failed | `run.local` |

`run` commands run once, in order, and stop at the first failure. `check`
commands must pass; health retries them until `timeout_secs`. A step takes
commands only where it can run them:

| Step | local | server | Server commands run in | Settings | If it fails |
| --- | --- | --- | --- | --- | --- |
| preflight | `run` | `run` | `dir` | `requires` | stops; nothing touched |
| build | `run` | ✘ | — | `version_from` | stops |
| pack | ✘ | ✘ | — | `binary`, `files` | stops |
| upload, unpack | ✘ | ✘ | — | — | the release is removed |
| setup | ✘ | `run`, `check` | `releases/<id>` | — | the release is removed; the old one keeps serving |
| backup | ✘ | ✘ | — | `files`, `keep` | the release is removed; the old one keeps serving |
| switch, restart | ✘ | ✘ | — | — | asks, then rolls back |
| health | `check` | `check` | `current` | `url`, `timeout_secs` | asks, then rolls back |
| done | `run` | `run` | `current` | — | a warning only |
| failed | `run` | ✘ | — | `rollback` | logged |

Local commands run in `project`. Every command is one shell line (`sh -c`;
on Windows, Git for Windows' `sh.exe`), so `&&`, pipes and `$VARS` work;
give one, or a list.

### Only what changed is sent

Pack hashes every file of the release (SHA-256) and asks the server for the
hashes of the same paths in the live release, computed there and then.
Files with the same path and hash stay out of the archive: unpack copies
them from `current` (keeping their mtimes, never as hard links, so the
release you would roll back to stays as it is) and gives them this
release's modes. Then `sha256sum -c` checks every file of the new release
against a receipt before it is moved into place. If a copied file does not
match, because the live release changed meanwhile, the release is removed
and sent whole once. With nothing live yet, or a server that cannot hash,
the whole release goes, as before. Folders, links, `release.json` and the
service file always go, and so do files whose names hold a newline, a
carriage return or a backslash.

```
✔ Pack    312 files · 309 already on the server · 3 to send · 25.0 MiB → 1.2 MiB
✔ Unpack  releases/v1.4.1-ab12cd3-20261015.120000 · 3 unpacked · 309 copied · verified
```

Each release's `release.json` also holds a fingerprint of what it is: its
files, modes and links, its service file (without the release's name) and
`shipdeck.yaml`. When the new release's fingerprint equals the live one's,
nothing changed, and shipdeck asks before making a duplicate:

```
╭ my-api · nothing changed ─────────────────────────────────────────────╮
│ Nothing changed since v1.4.0-ab12cd3-20261014.120000 (live).          │
│                                                                       │
│ Deploying again makes a duplicate release.                            │
│                                                                       │
│ Deploy anyway?                                                        │
│                                                                       │
│  y  deploy anyway     n  stop here                                    │
╰───────────────────────────────────────────────────────────────────────╯
```

`n`, Enter or Esc stop before anything on the server changes. Without the
TUI the answer is no, unless `--allow-unchanged`. An edited comment in
`shipdeck.yaml` counts as a change.

### Fields

| Field | Default | Meaning |
| --- | --- | --- |
| `name` | | Shown in the TUI and in the unit's `Description=` |
| `project` | this folder | Your source folder, relative to this `shipdeck.yaml` |
| `ssh.host` | | An address or a `~/.ssh/config` alias |
| `ssh.user` | alias `User` | Who to log in and deploy as. Required if the alias has no `User`. |
| `ssh.port` | alias `Port`, else `22` | |
| `ssh.shell_env` | `true` | Server commands get the environment a terminal has: shipdeck starts the login shell once per connection, so its startup files (`~/.profile` and the `~/.bashrc` it sources, `~/.zshrc`, `config.fish`) set up `PATH` (nvm and the like). `false` skips them. See [Tools on the server](#tools-on-the-server). |
| `service.name` | | The systemd unit, without `.service`; with PM2, also the app's name |
| `dir` | | The app's folder on the server, such as `/opt/my-app` |
| `service.manager` | `systemd` | What keeps the app running: `systemd` (a unit, installed with sudo) or `pm2` (run as `ssh.user`, no sudo). See [PM2](#pm2). |
| `service.pm2.app` | `service.name` | The app's name in PM2 |
| `service.pm2.ecosystem` | none | Your own PM2 file in the release, such as `ecosystem.config.js`, instead of the one shipdeck writes. `service.start` and `pm2.options` are then not used. |
| `service.pm2.options` | `{}` | More PM2 app settings for the file shipdeck writes: `instances: max`, `exec_mode: cluster`, `max_memory_restart: 300M`, ... |
| `service.start` | `./<binary>` | What the service runs, from the release folder: `./my-api -E=.env`, `node dist/main.js`, `.venv/bin/python -m app`. A bare program name is looked up on the server at preflight and written into the unit with its full path. Arguments follow the program; quote one with spaces: `--title "my api"`. |
| `service.run_as` | `ssh.user` | The user the service runs as. It must be able to read the release. Not with PM2, which runs the app as `ssh.user`. |
| `service.env_file` | none | A `KEY=value` file in the release, such as `.env`: the service's environment (`EnvironmentFile=`), and loaded for every server command too |
| `service.env` | `{}` | Variables for the service (`Environment=`) and for server commands |
| `service.shared` | `[]` | Folders kept across releases, such as `uploads`. Each lives in `shared/` and is linked into every release at unpack. |
| `pipeline.preflight.requires` | `[]` | Programs the server must have, such as `npm`. The program `service.start` names is checked anyway. |
| `pipeline.build.version_from` | found | The file whose version names the release, relative to `project`: `Cargo.toml` (`version.workspace = true` is followed), `package.json`, `pyproject.toml`, or any file whose first line is the version. Unset, the first of those in `project` (`VERSION` for the last), else the latest git tag, else `0.0.0`. |
| `pipeline.pack.binary` | none | A compiled program, relative to `project`. It lands at the release root, and its CPU and glibc floor are checked against the server. |
| `pipeline.pack.files` | none | What else ships; see [Files](#files) |
| `pipeline.backup.files.deployed` | `true` | The backup holds the files shipdeck shipped; see [Backups](#backups) |
| `pipeline.backup.files.include` | `[]` | More from the release, such as a file setup made that it cannot make again |
| `pipeline.backup.files.exclude` | `[]` | Left out of the backup, shipped or not |
| `pipeline.backup.keep.open` | `5` | Releases kept unpacked, counting the live one, ready for instant rollback |
| `pipeline.backup.keep.compressed` | `15` | Older releases kept only as their backup after that. `0` keeps none and makes none. |
| `pipeline.health.url` | none | Must answer 2xx, fetched on the server with curl or wget |
| `pipeline.health.timeout_secs` | `20` | How long the release has to become healthy, checks included |
| `pipeline.failed.rollback` | `ask` | `ask` or `auto`; see [When it fails](#when-it-fails-after-the-switch) |

### Files

```yaml
pack:
  binary: target/release/my-api       # optional: a compiled program → release root
  files:
    include:
      project: [dist, package.json]   # paths or globs, from project:
      deploy: [.env, config.yaml]     # paths or globs, next to shipdeck.yaml
    exclude:
      project: ["**/*.map", "dist/**/*.test.js"]
```

Every path keeps its place inside the release. Patterns are matched by
[fast-glob](https://github.com/oxc-project/fast-glob), the syntax of
fast-glob and Tailwind:

| Pattern | Matches |
| --- | --- |
| `dist` | the folder and everything in it |
| `*.js`, `?` | names inside one folder |
| `**/*.map` | at any depth, the top included |
| `src/*.{js,ts}`, `{lib,bin}/**` | one of several (braces nest) |
| `file[0-9].txt`, `[!_]*` | one character from a set, or not |
| `"!src/**/*.test.js"` | in `include`: leave these out. In `exclude`: keep these. |
| `dist/` | folders only |
| `.` | the whole source |
| `\*` | a literal `*` |

- Names starting with a dot match like any other; `.git` is never shipped.
- In `deploy`, `shipdeck.yaml` and `service.template` are never shipped.
- A file from `deploy` replaces a project file at the same path (the log says so).
- Quote a pattern that starts with `!` or `*`: YAML reads those as a tag or an alias.
- Every `include` pattern must find something after the build, or the deploy stops at Build (Preflight for `deploy`).
- `.env*` files always get mode `0600`; links inside folders stay links.

### Backups

After setup, shipdeck packs part of the new release into
`backups/<id>.tar.zst`. When the release gets older than `keep.open`, its
folder goes and the backup stays, up to `keep.compressed` of them.

```yaml
backup:
  files:
    deployed: true                    # the shipped files (the default)
    include: [data/keys]              # plus these, made by setup
    exclude: ["**/*.log"]             # minus these
  keep: { open: 5, compressed: 15 }
```

What setup makes, such as `node_modules` or `.venv`, stays out unless you
include it, so a backup is about the size of the upload. Rolling back to a
backup unpacks it, links `service.shared` and runs `pipeline.setup` again
(with `SHIPDECK_ACTION=rollback`), which puts the rest back; the server then
needs what setup needs, such as the npm registry. `include` and `exclude`
take the [Files](#files) patterns, relative to the release. `release.json`
and the service file are always in a backup; `service.shared` folders never
are.

### Variables

Every command sees these, besides its own environment:

| Variable | Value |
| --- | --- |
| `SHIPDECK_ACTION` | `deploy`, or `rollback` for a manual rollback |
| `SHIPDECK_STEP` | the step: `preflight`, `build`, ... In `failed`, the step that failed. |
| `SHIPDECK_ERROR` | in `failed`: the first line of the error |
| `SHIPDECK_RELEASE`, `SHIPDECK_VERSION` | the release going live, from build on (`v1.4.0-ab12cd3-…`, `1.4.0`) |
| `SHIPDECK_PREVIOUS` | the release that was live before, if any |
| `SHIPDECK_NAME`, `SHIPDECK_SERVICE`, `SHIPDECK_HOST` | `name`, `service.name`, `ssh.host` |
| `SHIPDECK_APP_DIR`, `SHIPDECK_SHARED_DIR` | `dir` and its `shared/` |
| `SHIPDECK_FOLDER`, `SHIPDECK_PROJECT` | local commands only: this deploy folder and `project`, absolute |

Server commands also get `service.env`, then `service.env_file` read the way
systemd reads it: `KEY=value` lines, `#` comments, one pair of quotes
removed, nothing expanded. Preflight reads it from the live release.

### When it fails after the switch

Once `current` points at the new release, a failing switch, restart or
health check puts the previous release back. With `failed.rollback: ask`
(the default) shipdeck asks first:

```
╭ web · health failed ─────────────────────────────────────╮
│                                                          │
│  v1.4.0 failed at Health: http 502 for 20s               │
│  v1.3.9 was live before it.                              │
│                                                          │
│  Roll back to v1.3.9?           rolling back in 27s      │
│                                                          │
│   y  roll back now        n  keep v1.4.0 live            │
╰──────────────────────────────────────────────────────────╯
```

No answer in 30 seconds rolls back. `n` keeps the new release live as it is,
to look at it; roll back later from the Releases screen. `rollback: auto`
rolls back at once without asking. Either way `failed` runs afterwards.

A manual rollback from the Releases screen goes switch → restart → health →
done, with `SHIPDECK_ACTION=rollback`, so `done` can tell everyone. A release
kept only as a backup is unpacked and set up first.

### Mistakes are caught before anything runs

```
✘ shipdeck did not start: 2 shipdeck.yaml files have 3 problems

web-prod/shipdeck.yaml
  error: unpack does not take commands
    ┌─ scripts/deploy/web-prod/shipdeck.yaml:36:3
    │
 36 │   unpack:
    │   ^^^^^^ unpack is done by shipdeck
    │
    = help: commands for the unpacked release go in pipeline.setup.run.server

  error: build cannot run commands on the server
    ┌─ scripts/deploy/web-prod/shipdeck.yaml:26:7
    │
 25 │     run:
 26 │       server: npm run build
    │       ^^^^^^ build runs on your PC only
    │
    = help: the server sets up the release in pipeline.setup.run.server

api-prod/shipdeck.yaml
  error: unknown key pipeline.pack.files.include.src
    ┌─ scripts/deploy/api-prod/shipdeck.yaml:21:9
    │
 21 │         src: [dist]
    │         ^^^ not a source
    │
    = help: sources are project and deploy
```

On a terminal it is in colour. Unknown keys get a "did you mean", and values that would be unsafe in a shell or a unit
(paths with `..`, odd service names, a relative `dir`) are refused.

### Editor completion

[`docs/schema/deploy.schema.json`](docs/schema/deploy.schema.json) describes every field, with the
same descriptions as above, for any editor using the YAML language server
(VS Code with the YAML extension, Neovim, Zed, JetBrains). Either start each
`shipdeck.yaml` with:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.7.1/docs/schema/deploy.schema.json
```

or map it once for the whole repository in `.vscode/settings.json`:

```json
{ "yaml.schemas": { "https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.7.1/docs/schema/deploy.schema.json": "**/shipdeck.yaml" } }
```

The website serves the newest one at
`https://shipdeck.naz.ovh/deploy.schema.json`.

## Tools on the server

An SSH command runs without a terminal, so the login shell skips the files
most tools are set up in: nvm adds itself to `~/.bashrc`, which stops early
when there is no terminal. So once per connection shipdeck starts the login
shell the way `ssh <host>` does for a terminal (`$SHELL -i -l`: bash, zsh,
fish, ... each read their own files) and gives every server command the
variables it exports. The shell's output and input are cut off, so a
startup file that prints or reads does no harm; one that fails, or takes
more than 20 s, is reported at preflight, and commands run without it.

Everything shipdeck looks for on the server is found this way: `pm2`,
`node`, `npm`, a bare `service.start` program and `preflight.requires`.
Which files are read depends on the user's login shell, as for a terminal:

| Shell | Reads |
| --- | --- |
| bash | `/etc/profile`, then the first of `~/.bash_profile`, `~/.bash_login`, `~/.profile`. `~/.bashrc` only when that file sources it (Debian's and Ubuntu's `~/.profile` do). |
| zsh | `~/.zshenv`, `~/.zprofile`, `~/.zshrc`, `~/.zlogin` |
| fish | `~/.config/fish/config.fish` and `conf.d` |

So nvm set up in `~/.bashrc` is missed when a `~/.bash_profile` does not
source `~/.bashrc`. To see what shipdeck sees, run the same thing yourself:

```sh
ssh deploy@prod '"$SHELL" -i -l -c "command -v pm2; pm2 -v"'
```

A plain `ssh deploy@prod pm2 -v` reads none of those files, so it can say
"command not found" while shipdeck finds `pm2`.

It costs a second or two per connection. To skip it, set
`ssh.shell_env: false` for one server, or pass `--no-shell-env` for a whole
run. Something in the startup files that expects a person, such as starting
tmux, is a reason to.

The built-in unit is hardened (`ProtectHome=true` among others), so the
service cannot see `/home`. A runtime installed per user, such as Node from
`nvm`, is therefore refused at preflight with a hint: install it system-wide
(`apt`, NodeSource, `/usr/local/bin`), or use a `service.template` without
`ProtectHome`. Server commands are not affected, since they run over SSH.

## PM2

With `service.manager: pm2`, PM2 keeps the app running instead of systemd.
It runs as `ssh.user`, so the server needs no sudo rule, and a runtime from
`nvm` works.

```yaml
dir: /opt/web                             # current/ releases/ backups/ shared/
service:
  name: web
  manager: pm2
  start: node dist/main.js
  env_file: .env
  pm2:
    options: { instances: max, exec_mode: cluster }
```

Each release carries `web.pm2.json`, with `cwd` set to `current`. The
restart step runs, in `current` and with `service.env` and
`service.env_file` loaded:

```sh
pm2 startOrReload <dir>/current/web.pm2.json --only web --update-env
pm2 save
```

so a rollback reloads the old release's file, and the app comes back after
a reboot. `node <script>` runs the script with node as PM2's interpreter, so
cluster mode works; node's own flags go in `pm2.options.node_args`. Any
other program runs as it is. To keep your own file
instead, ship it and name it:

```yaml
  pm2:
    app: web                          # the app in the file to reload
    ecosystem: ecosystem.config.js    # or .config.cjs, .json, .yml
```

Health reads `pm2 jlist`: the app is up when every instance is `online`,
and a rising restart count is a crash. A failure shows `pm2 logs`.

Preflight takes any PM2 version, since shipdeck uses only these basic commands; one
newer than the tested 7.x gets a warning at preflight, and the deploy goes on.
Preflight refuses a systemd unit of the same name that is still running (`sudo
systemctl disable --now <name>` first). `server-setup.sh --manager pm2`
skips the sudo rule and runs `pm2 startup` for the deploy user, once PM2 is
installed. PM2 7 requires Node.js 18 or newer. To install the tested release,
run `npm install -g pm2@7.0.4`; when upgrading an existing PM2 installation,
run `pm2 update` as the deploy user to update its running daemon too.

## service.template

Without one, shipdeck uses a hardened unit: `Restart=always`, read-only
system, writes allowed only to `<dir>/shared`. To change it, copy that unit
from `crates/engine/src/bundle/default.service` into `service.template` and
edit it. Placeholders:

`{{name}}` `{{tag}}` `{{id}}` `{{version}}` `{{git}}` `{{dir}}` `{{service}}`
`{{user}}` `{{exec}}` (the whole `ExecStart=` command) `{{binary}}`
`{{env}}` (the `EnvironmentFile=` and `Environment=` lines)
`{{workdir}}` (the live working folder)

An unknown placeholder stops the deploy at Preflight; `{{binary}}` does too
when there is no `pipeline.pack.binary`.

## The agent

On each connection the engine checks the installed `shipdeck-agent` version
and starts it through sudo. The account must be root or have password-free
sudo. Published builds download the matching release from GitHub into
`/opt/shipdeck/agent/v<version>/agent`; root owns the binary and its parent
directories. Same-major minor and patch differences are allowed. A newer agent
major refuses the connection. Development builds use the explicit
`SHIPDECK_AGENT_BINARY` path, and a failed download never falls back to a local
binary. The agent opens no port and exits when the connection ends.

The agent does the file and service work with typed requests: hashing reusable
files, receiving the bundle on a separate SSH channel, safe tar/zstd
extraction with path, link, mode, size and checksum checks, shared links,
backups and restores, release listing and inspection, service status and
control, live logs, project locks, and approved cleanup. Deploys still run
while connected: prepare and verify first, activate last. `~/.shipdeck/state.db`
keeps numbered events, and `shipdeck events` replays them.

When an operation is interrupted, unfinished files stay. `shipdeck work`
lists them and the exact service state; `--cleanup` removes only that list,
after the check that the server still reports each path. A dropped connection
during activation is never reported as success: reconnect and inspect the
live release and service with `shipdeck work`.

One project lock covers deploys, rollbacks, cleanup and service control. The
connected agent holds the lock. Upload tickets are tied to the lock token, and
every `shipdeck.yaml` server command writes a `.shipdeck.command` lease. A new
operation refuses while that command is alive, so an agent disconnect cannot
let two deploys overlap.

If agent preparation or the handshake fails, the connection stops. The engine
does not use a plain SSH path to bypass the version or privilege rules.

dir: /opt/web
