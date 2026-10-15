# shipdeck

A terminal UI that builds an app on your own machine, ships it to a server
over SSH, and runs it there as a systemd service. Every deploy is kept as its
own release, so any of them can go live again from the Releases screen.

It exists for servers too small to build on. Whatever the language (Rust, Go,
Node.js, Python, ...), the build happens locally and only what you list
travels. Each step of the pipeline can run your own commands, and
`deploy.yaml` says plainly which ones run on your PC and which on the server.

## Get started

**1. The server.** shipdeck logs in as `ssh.user`, whoever you choose. Nothing
is set up beforehand: each app's first deploy creates its folder and installs
its service. If that user lacks a [permission](#permissions) it needs, the
deploy stops at preflight and says which, before anything changes.

**2. Write a `deploy.yaml`** in one folder per deployable, such as
`scripts/deploy/my-api/deploy.yaml`. Start from [`examples/`](examples); every
field is under [deploy.yaml](#deployyaml).

**3. Deploy.** Run `shipdeck ./scripts/deploy`, pick the deployable and press
`⏎`.

## Install

```sh
# Linux and macOS: the latest release, or the one -V names
curl -LsSf https://shipdeck.naz.ovh/install.sh | sh
curl -LsSf https://shipdeck.naz.ovh/install.sh | sh -s -- -V=v0.4.0

# from source
cargo install --git https://github.com/Naz-Ovh/shipdeck shipdeck
```

The script installs to `~/.local/bin`. On Linux it picks the gnu build when
the system has glibc 2.28 or newer, and a static musl build otherwise
(Alpine, or an older distribution).

## Run it

```sh
cargo run --release -p shipdeck -- <deploy-root> [--key <path> | --password <value>]

shipdeck ./scripts/deploy --key ~/.ssh/id_ed25519
shipdeck ./scripts/deploy                 # asks for the password in the TUI
shipdeck --schema > deploy.schema.json    # the JSON Schema, for offline editors
```

| Flag | Login |
| --- | --- |
| `--key <path>` | Private key file. An encrypted key asks for its passphrase in the TUI. |
| `--password <value>` | Password. It lands in your shell history; in zsh with `HIST_IGNORE_SPACE`, start the command with a space. `SHIPDECK_PASSWORD` does the same without the history. |
| neither | The host's `IdentityFile` from `~/.ssh/config` if it has one; if there is none or the server refuses it, the TUI asks for the password once per `user@host` and keeps it in memory for the run. |

### Without the TUI

For scripts and CI, three commands do the same work with no screen. They
print progress to stderr as plain lines and never wait for an answer.

```sh
shipdeck deploy   ./scripts/deploy my-api [my-web ...] [--on-failure rollback|keep]
shipdeck rollback ./scripts/deploy my-api --to v0.3.0-9f8e7d6-20261012.091000
shipdeck releases ./scripts/deploy my-api     # the table on stdout, newest first
```

A deployable is named by its folder or its `name`. `deploy` takes several and
runs them one after another; the first that does not succeed stops the rest.
Every `deploy.yaml` under the root is still checked first.

What the TUI would ask comes from flags instead:

| The TUI asks | Without the TUI |
| --- | --- |
| Deploy this? | Running the command is the yes. |
| Roll back after a failed switch, restart or health check? | `--on-failure rollback` (the default) rolls back at once; `--on-failure keep` keeps the failed release live. Either overrides `pipeline.health.rollback`. |
| Trust a server seen for the first time? | No, unless `--trust-new-host`. A changed host key is always refused. |
| A password or key passphrase? | Never asked: use `--key` with a key without a passphrase, `--password` or `SHIPDECK_PASSWORD`. |

The exit code says how it ended:

| Code | Meaning |
| --- | --- |
| `0` | The release is live. |
| `1` | It failed before the live release changed, or shipdeck could not start (a config problem, an unknown name). |
| `2` | Wrong usage (from the flag parser). |
| `3` | The new release failed and the previous one is live again. |
| `4` | The new release failed and stays live (`--on-failure keep`). |
| `5` | The new release failed and putting the old one back failed too: look at the server now. |
| `130` | Ctrl-C twice. The first one only warns, since stopping can leave the server between two releases. |

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
│   ├── config.yaml          optional: shipped with pipeline.pack.files.include.deploy
│   └── .env                 gitignored
└── web-prod/
```

shipdeck reads every `deploy.yaml` before it starts. If any of them has a
mistake, it does not start, and prints every problem in every file at once
(see [Mistakes](#mistakes-are-caught-before-anything-runs)).

## deploy.yaml

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.4.0/deploy.schema.json
name: Web (prod)
project: ../../../web                       # your source; local commands run here
ssh:
  host: prod                                # address or ~/.ssh/config alias
  user: deploy
service:
  name: web                                 # web.service
  dir: /opt/web                             # current/ releases/ backups/ shared/
  start: node dist/main.js                  # default: ./<binary>
  env_file: .env                            # also loaded for server commands
  env: { NODE_ENV: production }
  shared: [uploads]                         # kept across releases
  keep: { open: 5, compressed: 15 }
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
        deploy: [.env]                      # next to this deploy.yaml
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
  # switch, restart                         shipdeck
  health:
    url: http://127.0.0.1:3000/health       # fetched on the server
    check:
      server: ./bin/smoke-test
      local: curl -fsS https://web.example.com/health
    timeout_secs: 20
    rollback: ask                           # default; or auto
  done:                                     # new release is live
    run:
      server: ./bin/warm-cache
      local: ./notify.sh "web $SHIPDECK_VERSION is live"
  failed:
    run:
      local: ./notify.sh "failed at $SHIPDECK_STEP"
```

[`examples/`](examples) has complete files for [Rust](examples/rust/deploy.yaml),
[Go](examples/go/deploy.yaml), [Node.js](examples/node/deploy.yaml) and
[Python](examples/python/deploy.yaml); a test keeps them valid.

### The steps

| # | Step | shipdeck does | your commands |
| --- | --- | --- | --- |
| 1 | preflight | checks your PC and the server | `run.server` → `run.local` |
| 2 | build | names the release | `run.local` |
| 3 | pack | packs the binary and files | — |
| 4 | upload | sends the archive to the server | — |
| 5 | unpack | unpacks it into `releases/<id>`, links `service.shared` | — |
| 6 | setup | nothing: the old release is still live | `run.server` → `check.server` |
| 7 | switch | points `current` at `releases/<id>`, installs its unit | — |
| 8 | restart | restarts the service | — |
| 9 | health | waits until the service is active and `url` answers | `check.server` → `check.local` |
| 10 | done | tidies old releases (`service.keep`) | `run.server` → `run.local` |
| ✱ | failed | a step failed | `run.local` |

`run` commands run once, in order, and stop at the first failure. `check`
commands must pass; health retries them until `timeout_secs`. A step takes
commands only where it can run them:

| Step | local | server | Server commands run in | Settings | If it fails |
| --- | --- | --- | --- | --- | --- |
| preflight | `run` | `run` | `service.dir` | `requires` | stops; nothing touched |
| build | `run` | ✘ | — | `version_from` | stops |
| pack | ✘ | ✘ | — | `binary`, `files` | stops |
| upload, unpack | ✘ | ✘ | — | — | the release is removed |
| setup | ✘ | `run`, `check` | `releases/<id>` | — | the release is removed; the old one keeps serving |
| switch, restart | ✘ | ✘ | — | — | asks, then rolls back |
| health | `check` | `check` | `current` | `url`, `timeout_secs`, `rollback` | asks, then rolls back |
| done | `run` | `run` | `current` | — | a warning only |
| failed | `run` | ✘ | — | — | logged |

Local commands run in `project`. Every command is one shell line (`sh -c`),
so `&&`, pipes and `$VARS` work; give one, or a list.

### Fields

| Field | Default | Meaning |
| --- | --- | --- |
| `name` | | Shown in the TUI and in the unit's `Description=` |
| `project` | this folder | Your source folder, relative to this `deploy.yaml` |
| `ssh.host` | | An address or a `~/.ssh/config` alias |
| `ssh.user` | alias `User` | Who to log in and deploy as. Required if the alias has no `User`. |
| `ssh.port` | alias `Port`, else `22` | |
| `service.name` | | The systemd unit, without `.service` |
| `service.dir` | | The app's folder on the server, such as `/opt/my-app` |
| `service.start` | `./<binary>` | What the service runs, from the release folder: `./my-api -E=.env`, `node dist/main.js`, `.venv/bin/python -m app`. A bare program name is looked up on the server at preflight and written into the unit with its full path. |
| `service.run_as` | `ssh.user` | The user the service runs as. It must be able to read the release. |
| `service.env_file` | none | A `KEY=value` file in the release, such as `.env`: the service's environment (`EnvironmentFile=`), and loaded for every server command too |
| `service.env` | `{}` | Variables for the service (`Environment=`) and for server commands |
| `service.shared` | `[]` | Folders kept across releases, such as `uploads`. Each lives in `shared/` and is linked into every release at unpack. |
| `service.keep.open` | `5` | Releases kept unpacked, counting the live one, ready for instant rollback |
| `service.keep.compressed` | `15` | Older releases kept as `.tar.gz` after that |
| `pipeline.preflight.requires` | `[]` | Programs the server must have, such as `npm`. The program `service.start` names is checked anyway. |
| `pipeline.build.version_from` | found | The file whose version names the release, relative to `project`: `Cargo.toml` (`version.workspace = true` is followed), `package.json`, `pyproject.toml`, or any file whose first line is the version. Unset, the first of those in `project` (`VERSION` for the last), else the latest git tag, else `0.0.0`. |
| `pipeline.pack.binary` | none | A compiled program, relative to `project`. It lands at the release root, and its CPU and glibc floor are checked against the server. |
| `pipeline.pack.files` | none | What else ships; see [Files](#files) |
| `pipeline.health.url` | none | Must answer 2xx, fetched on the server with curl or wget |
| `pipeline.health.timeout_secs` | `20` | How long the release has to become healthy, checks included |
| `pipeline.health.rollback` | `ask` | `ask` or `auto`; see [When it fails](#when-it-fails-after-the-switch) |

### Files

```yaml
pack:
  binary: target/release/my-api       # optional: a compiled program → release root
  files:
    include:
      project: [dist, package.json]   # paths or globs, from project:
      deploy: [.env, config.yaml]     # paths or globs, next to deploy.yaml
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
- In `deploy`, `deploy.yaml` and `service.template` are never shipped.
- A file from `deploy` replaces a project file at the same path (the log says so).
- Quote a pattern that starts with `!` or `*`: YAML reads those as a tag or an alias.
- Every `include` pattern must find something after the build, or the deploy stops at Build (Preflight for `deploy`).
- `.env*` files always get mode `0600`; links inside folders stay links.

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
| `SHIPDECK_APP_DIR`, `SHIPDECK_SHARED_DIR` | `service.dir` and its `shared/` |
| `SHIPDECK_FOLDER`, `SHIPDECK_PROJECT` | local commands only: this deploy folder and `project`, absolute |

Server commands also get `service.env`, then `service.env_file` read the way
systemd reads it: `KEY=value` lines, `#` comments, one pair of quotes
removed, nothing expanded. Preflight reads it from the live release.

### When it fails after the switch

Once `current` points at the new release, a failing switch, restart or
health check puts the previous release back. With `rollback: ask` (the
default) shipdeck asks first:

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
done, with `SHIPDECK_ACTION=rollback`, so `done` can tell everyone.

### Mistakes are caught before anything runs

```
✘ shipdeck did not start: 2 deploy.yaml files have 3 problems

web-prod/deploy.yaml
  error: unpack does not take commands
    ┌─ scripts/deploy/web-prod/deploy.yaml:36:3
    │
 36 │   unpack:
    │   ^^^^^^ unpack is done by shipdeck
    │
    = help: commands for the unpacked release go in pipeline.setup.run.server

  error: build cannot run commands on the server
    ┌─ scripts/deploy/web-prod/deploy.yaml:26:7
    │
 25 │     run:
 26 │       server: npm run build
    │       ^^^^^^ build runs on your PC only
    │
    = help: the server sets up the release in pipeline.setup.run.server

api-prod/deploy.yaml
  error: unknown key pipeline.pack.files.include.src
    ┌─ scripts/deploy/api-prod/deploy.yaml:21:9
    │
 21 │         src: [dist]
    │         ^^^ not a source
    │
    = help: sources are project and deploy
```

On a terminal it is in colour. Unknown keys get a "did you mean", a 0.2 key
says where it moved, and values that would be unsafe in a shell or a unit
(paths with `..`, odd service names, a relative `service.dir`) are refused.

### Editor completion

[`deploy.schema.json`](deploy.schema.json) describes every field, with the
same descriptions as above, for any editor using the YAML language server
(VS Code with the YAML extension, Neovim, Zed, JetBrains). Either start each
`deploy.yaml` with:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.4.0/deploy.schema.json
```

or map it once for the whole repository in `.vscode/settings.json`:

```json
{ "yaml.schemas": { "https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v0.4.0/deploy.schema.json": "**/deploy.yaml" } }
```

### From 0.2

shipdeck 0.3 refuses a 0.2 file and says where each key went:

| 0.2 | 0.3 |
| --- | --- |
| `build.cwd` | `project` |
| `build.command` | `pipeline.build.run.local` (a list is now several commands; write `[cargo, build]` as one line) |
| `build.binary` | `pipeline.pack.binary` |
| `build.files` | `pipeline.pack.files.include.project` |
| `build.version_from` | `pipeline.build.version_from`, now relative to `project` |
| `bundle` | `pipeline.pack.files.include.deploy` |
| `remote.dir`, `remote.service` | `service.dir`, `service.name` |
| `remote.start` + `remote.args` | `service.start`, arguments included |
| `remote.env`, `remote.env_file`, `remote.run_as` | `service.env`, `service.env_file`, `service.run_as` |
| `remote.requires` | `pipeline.preflight.requires` |
| `releases` | `service.keep` |
| `health` | `pipeline.health` |
| `hooks.before` | `pipeline.preflight.run.local` |
| `hooks.server` | `pipeline.setup.run.server` |
| `hooks.after` | `pipeline.done.run.local` |

A `deploy.json` from 0.1 is refused too; JSON is YAML, so rename it and move
the keys the same way.

## Tools on the server

The built-in unit is hardened (`ProtectHome=true` among others), so the
service cannot see `/home`. A runtime installed per user, such as Node from
`nvm`, is therefore refused at preflight with a hint: install it system-wide
(`apt`, NodeSource, `/usr/local/bin`), or use a `service.template` without
`ProtectHome`. Server commands are not affected, since they run over SSH.

## service.template

Without one, shipdeck uses a hardened unit: `Restart=always`, read-only
system, writes allowed only to `<dir>/shared`. To change it, copy that unit
from `crates/engine/src/bundle/default.service` into `service.template` and
edit it. Placeholders:

`{{name}}` `{{tag}}` `{{id}}` `{{version}}` `{{git}}` `{{dir}}` `{{service}}`
`{{user}}` `{{exec}}` (the whole `ExecStart=` command) `{{binary}}`
`{{env}}` (the `EnvironmentFile=` and `Environment=` lines)

An unknown placeholder stops the deploy at Preflight; `{{binary}}` does too
when there is no `pipeline.pack.binary`.

## Permissions

shipdeck deploys as `ssh.user`, whoever that is. On the server, that user
must be able to:

| What | Why |
| --- | --- |
| create `service.dir` (itself, or with sudo), or write to it | the first deploy creates the app's folder; every deploy writes releases into it |
| `systemctl daemon-reload`, `systemctl enable <service>`, `systemctl restart <service>` | install and restart the app |
| `install -m 0644 <dir>/current/<service>.service /etc/systemd/system/<service>.service` | put the release's unit in place |
| `journalctl -u <service> -n 30 --no-pager` | show the log when a release fails |

As root these simply run. Any other user runs the last three through
`sudo -n`, so it needs sudo without a password for them. Preflight checks
each one and, when something is missing, stops and names it.

## On the server

```
/opt/my-app/
├── current -> releases/v0.3.1-ab12cd3-20261014.153012
├── releases/        newest `keep.open` releases, unpacked
├── backups/         next `keep.compressed` releases, as .tar.gz
└── shared/          kept across deploys; the only folder the default unit may write to
```

A release id is `v<version>-<git sha>[-dirty]-<UTC yyyymmdd.hhmmss>`. Each
release carries `release.json` (who deployed what, from which commit) and its
own unit, whose `Description=` holds the version, so `systemctl status`
shows what is live. Each release also installs its own dependencies in
setup, so a rollback finds them already in place.

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

When a release fails after the switch, the rollback question takes `y` (roll
back now) or `n` (keep it live).

## Development

```
bin/shipdeck/      the flags; starts the TUI or a headless command
crates/tui/        the TUI: screens, dialogs, key handling
crates/headless/   deploy, rollback and releases without the TUI
crates/engine/     everything else: config, build, bundle, ssh, remote, deploy flows
docs/plan.md       the design and the decisions behind it
```

The engine knows nothing about the screen; it reports through
`deploy::report::Reporter`, and the TUI and headless crates each show those
reports their own way. Changes to the TUI recompile only `crates/tui` and the
small bin.

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
SHIPDECK_DUMP=/tmp/frames cargo test -p shipdeck-tui   # writes every screen as text
```

There is no CI for these: [lefthook](https://lefthook.dev) runs them on your
machine. Install it once (for example `go install
github.com/evilmartians/lefthook/v2@latest`), then run `lefthook install` in
each clone. Every commit checks that the README, the examples and
`deploy.schema.json` name the version in `Cargo.toml`. Every push runs fmt,
check, clippy and test, and refuses a `v<version>` tag whose commit has
another version in `Cargo.toml`. The scripts are in `scripts/lefthook/`.

The config types in `crates/engine/src/config/mod.rs` are the schema: their
doc comments become the editor descriptions, and the launch check walks the
same schema. After changing them, regenerate the file (a test fails until
you do):

```sh
cargo run -q -p shipdeck -- --schema > deploy.schema.json
```

### Releasing

The setup command fetches `server-setup.sh`, and editors fetch
`deploy.schema.json`, at the tag `v<version>`, so every release needs its tag
pushed. Bump `[workspace.package] version` in `Cargo.toml`, update the schema
line in `examples/` and this README, regenerate `deploy.schema.json`, commit,
and push the tag:

```sh
git tag v0.4.0 && git push origin main v0.4.0
```

`setup_test.rs` fails if `server-setup.sh` and `remote::sudo_commands` ever
disagree, and `config_test.rs` fails if the examples point at another
version's schema.

The tag push runs `.github/workflows/release.yml`, which
[dist](https://github.com/axodotdev/cargo-dist) generates from
`dist-workspace.toml`. Nothing else runs it, pull requests included. It builds
six binaries (gnu and static musl for x86_64 and aarch64 Linux, plus both
macOS architectures) and checks that no gnu binary needs a glibc newer than
`min-glibc-version`. Only then does it create the GitHub Release, with the
archives, their checksums and `shipdeck-installer.sh` attached, so a release
is never public without its binaries. If the build or the check fails, no
release is made. The docs site deploys once the release is published. Do not
make the release on GitHub by hand: the workflow then fails, because the
release already exists. The release notes come from dist; edit them on GitHub
afterwards if you like.

After changing `dist-workspace.toml`, run `dist generate`: the release
workflow refuses to run while `release.yml` is out of date.
