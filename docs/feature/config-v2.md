# Config redesign

Status: proposal, 2026-11-10. Not agreed. No code has changed. Shipdeck is in
alpha, so this is a clean break: old configs are refused, with no migration. Their keys show up as
unknown keys.

Files:

- [reference.yaml](config-v2/reference.yaml): a deploy file with every key and
  its default.
- [standalone.yaml](config-v2/standalone.yaml): a deploy with one app, every
  key it can use.
- [universabile/](config-v2/universabile/): a real project in this format.

## Why

Today's config is hard to predict:

- Shipdeck scans a folder for subfolders that hold a `shipdeck.yaml`. A deploy
  is a folder, with app files, a unit template and other files beside it.
- The top `pipeline` means two things. Its commands run once, but its settings
  (`pack`, `health`) are copied into every app.
- Paths start from different places: the project, the deploy folder, or the
  app's project.
- Nothing shows the server tree before a deploy.

## The rules

1. **No scanning.** Shipdeck reads one root file in the project folder, the
   one `-c` names, or the one deploy file you name.
2. **One deploy is one file.** No folder, no app files, no files found by name.
3. **Three path forms, nothing else.**
4. **Two levels, no copying.** `release:` runs or applies once. `apps.<app>:`
   is that app only.
5. **Every step has `local:` and `server:`.** `local` runs on your PC.
   `server` runs on the server.
6. **Every file lands at a path you can work out from the config.**
7. **`shipdeck tree` prints the full release before you deploy.** It does not
   connect to the server.

## Files

### The root file

The root file lists every deploy of the project. Shipdeck reads
`shipdeck.yaml` or `.shipdeck` in the project folder, or the file
`--config` (`-c`) names. Having both names in one folder is an error.

```yaml
root: true
deploys:
  production: scripts/deploy/production.yaml   # a deploy file
  staging:                                      # or the deploy written here
    name: shop
    server: { ssh: { host: staging }, dir: /opt/shop }
    apps:
      web: { source: apps/web, files: [apps/web/dist], static: true }
```

- `root: true` is required. It marks the file as a root file.
- `deploys.<name>` is a deploy file's path, or the deploy written in place.
  A deploy written in place follows every rule of a deploy file. Its `./x`
  paths start from the root file's folder.
- A deploy name uses letters, digits, `-` and `_`. It never holds a dot, so
  it never looks like a file name.

### The deploy file

One file holds one whole deploy: the server, the release and every app. It
can live anywhere. Its name ends in `.yaml` or `.yml`. It never holds `root`
or `deploys`.

### The project folder

The project folder is the folder you run shipdeck in, or the one `--cwd`
(`-C`) names. `apps/x` paths start there, and `{{PROJECT}}` is it.

`-C` applies first, as in `git -C`: every relative path on the command line,
including `-c` and a deploy file, starts from it. Nothing else changes the
project folder: not `-c`, and not a deploy file's place.

### Choosing a deploy

A command takes a deploy name or a deploy file:

| Written | Read as |
| --- | --- |
| `production` | A deploy name, from the root file. |
| `production.yaml`, `scripts/deploy/prod.yml` | A deploy file. No root file is read. |
| `./production`, `scripts/deploy/prod` | An error: a deploy file ends in `.yaml` or `.yml`. |

Only the ending decides. A `/` is part of the path and nothing more.

| Command | Reads |
| --- | --- |
| `shipdeck` | The root file; the TUI lists its deploys. |
| `shipdeck deploy production` | The root file, then `production`. |
| `shipdeck deploy scripts/deploy/production.yaml` | That deploy file only. |
| `shipdeck -c ops/shipdeck.yaml deploy production` | The root file at that path, then `production`. |
| `shipdeck -C ~/code/shop deploy production` | As if run in `~/code/shop`. |
| `shipdeck deploy api ./staging.yaml` | Names and files mixed, one after another. |

`-c` always names a file, so it needs no ending. It accepts `.shipdeck`.

Errors:

| Case | Message |
| --- | --- |
| No root file, and no deploy file named | Names the folder and the two root file names it expected. |
| `-c` names a file without `root: true` | "This is a deploy file; run `shipdeck deploy <file>`." |
| A deploy file argument holds `root: true` | "This is a root file; use `-c <file>`." |
| A name the root file does not list | Names the root file, and says a file needs `.yaml` or `.yml`. |

## Paths on your PC

Every path to a file on your PC uses one of three forms:

| Written | Means | Example |
| --- | --- | --- |
| `/opt/x` | Absolute. | `/home/me/keys/api.pem` |
| `apps/admin` | From the **project folder**: where you run shipdeck, or `-C`. | `apps/admin/dist` |
| `./x` or `../x` | From the folder of the config file that holds it. | `./production/.env` |

This covers `deploys`, `source`, `files`, `binary`, `version_from` and
`systemd.template`. Patterns in `exclude` match the path from the project
folder.

**Paths in the release** are different. They are paths on the server, inside a
release: `to`, `nginx.root`, `env_file`, `shared`, `start` and
`pm2.ecosystem`. They never point at your PC.

## Where files land

`<id>` is the release name. `current` points at the live release.

| What | Lands at |
| --- | --- |
| `release.files`, project path | `releases/<id>/<path from the project folder>` |
| `apps.<app>.files`, inside `source` | `releases/<id>/apps/<app>/<path inside source>` |
| Any file with `to:` | `releases/<id>/<to>`, or `releases/<id>/apps/<app>/<to>` in an app |
| `apps.<app>.binary` | `releases/<id>/apps/<app>/<file name>`, or `apps/<app>/<to>`; mode `0755` |
| Shipdeck's own files | `releases/<id>/.shipdeck/` |

Terms:

- **source**: the app's local source directory, on your PC. Its files keep
  their path inside it, and the app's `local` commands run in it. Default: the
  project folder.
- **app folder**: `apps/<app>` in the release. Every app lands there, even
  when the deploy has only one app. There is no setting to move it.

A file without a place of its own needs `to:`. These are refused without it:

- a `./`, `../` or absolute path in `release.files`;
- an app file outside the app's `source`.

```yaml
apps:
  api:
    source: services/api
    files:
      - services/api/dist                      # → apps/api/dist
      - { path: services/api/dist/server, to: server }   # → apps/api/server
      - { path: ./prod/api.conf, to: api.conf }          # → apps/api/api.conf
```

`binary` takes the same two forms as a file entry, and lands in the app
folder:

- `binary: target/release/api` lands at `apps/api/api`.
- `binary: { path: target/release/api, to: bin/api }` lands at
  `apps/api/bin/api`.

`to` is optional here, since a binary always has a place: its file name.
The path must be one file, not a folder. It may be anywhere on your PC.

### Shipdeck's own files

Everything shipdeck writes goes in `.shipdeck/`. The rest of the release is
only your files.

| File | Written when |
| --- | --- |
| `.shipdeck/release.json` | Always. |
| `.shipdeck/settings.json` | Always. The resolved config, for rollback. |
| `.shipdeck/<process>.service` | A systemd app. |
| `.shipdeck/<process>.pm2.json` | A PM2 app without its own ecosystem file. |
| `.shipdeck/oxfile.toml` | An oxmgr app, once oxmgr is built. Not written today. |
| `.shipdeck/nginx/<name>-<app>.conf` | An app with `nginx:`, once nginx is built. Not written today. |

Keeping the nginx file in the release, once it is written, lets a rollback restore
routing too. Until then a deploy with `nginx:` or `oxmgr:` stops at preflight, so the
plan holds no placeholder for either.

### Shared folders

A shared folder keeps its data across releases. The link is in the release;
the data is in `shared/` at the same path:

| Config | Link | Data |
| --- | --- | --- |
| `release.shared: [storage]` | `releases/<id>/storage` | `shared/storage` |
| `apps.website.shared: [cache]` | `releases/<id>/apps/website/cache` | `shared/apps/website/cache` |

### Errors instead of guesses

- **One path, one source.** The same file picked twice lands once. Two
  different files for one path is an error that names both.
- Nothing may be packed into `.shipdeck/` or into a shared folder.
- A path in the release may not start with `/` or hold `..`.
- A pattern that matches nothing is an error, as today.

## Steps

### The rule: your commands, then the action

Every step has two parts, always in this order:

1. **Your commands.** `server:` runs on the server, then `local:` runs on your
   PC. Release commands run first, then each app's, in app-name order.
2. **Shipdeck's action**, the work the step exists for.

Your commands are always *before* the action. If one fails, the action does
not run, and the step fails.

Every step takes `local:` and `server:`, under `release.steps` and under
`apps.<app>.steps`. Leave a key out when there is nothing to run.

### The 12 steps, in order

| # | Step | When | Shipdeck's action, after your commands |
| --- | --- | --- | --- |
| 1 | `preflight` | every deploy | Checks the SSH connection, `requires`, runners and permissions. |
| 2 | `build` | every deploy | Collects and hashes `files`. A pattern that matches nothing fails the step. |
| 3 | `upload` | every deploy | Packs `files`, sends them, unpacks them into `{{RELEASE}}`, links shared folders. |
| 4 | `setup` | every deploy | None. The step is your commands. |
| 5 | `backup` | every deploy | Writes `backups/<id>.tar.zst`. |
| 6 | `switch` | every deploy | Points `current` at the new release. |
| 7 | `restart` | every deploy | Restarts each process. A static app has nothing to restart. |
| 8 | `nginx` | every deploy | Writes, checks and reloads nginx for apps with `nginx:`, and gets a missing certificate. |
| 9 | `health` | every deploy | Fetches `health.url`. |
| 10 | `done` | every deploy | Removes old releases beyond `backup.keep`. |
| 11 | `rollback` | a failure in steps 6–9, or by hand | Puts an older release back. See [Rollback](#rollback). |
| 12 | `failed` | any failure | None. The step is your commands. |

Notes:

- **build:** the release is named when `build` starts, before your commands.
  So `{{RELEASE}}` and `{{RELEASE_ID}}` work from `build` on.
- **health:** each try runs your commands, then fetches `health.url`. A
  failure in either starts the next try, until `timeout_secs`. Apps are
  checked first; the release commands run once every app is healthy.
- **failed:** `failed.server` runs only when the server can be reached. When
  it can't, the log says the commands were skipped.
- `setup.check` is gone: list the checks after the commands in `setup.server`.
- There is no `pack` step. What ships is `files`.

### Settings

| Step | Settings |
| --- | --- |
| `preflight` | `requires` (release only) |
| `build` | `version_from` (release only) |
| `backup` | `keep` (release only), `include`, `exclude` |
| `health` | `timeout_secs` (release only), `url` (app only) |
| `rollback` | `mode`, `ask_secs`, `keep_failed` (release only) |

### Where your commands run

| Step | Release `server` | App `server` | Release `local` | App `local` |
| --- | --- | --- | --- | --- |
| preflight | `{{ROOT}}` | `{{ROOT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| build | `{{ROOT}}` | `{{ROOT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| upload | `{{ROOT}}` | `{{ROOT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| setup | `{{RELEASE}}` | `{{APP_RELEASE}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| backup | `{{RELEASE}}` | `{{APP_RELEASE}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| switch | `{{RELEASE}}` | `{{APP_RELEASE}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| restart | `{{CURRENT}}` | `{{APP_CURRENT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| nginx | `{{CURRENT}}` | `{{APP_CURRENT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| health | `{{CURRENT}}` | `{{APP_CURRENT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| done | `{{CURRENT}}` | `{{APP_CURRENT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| rollback | `{{ROOT}}` | `{{ROOT}}` | `{{PROJECT}}` | `{{SOURCE}}` |
| failed | `{{ROOT}}` | `{{ROOT}}` | `{{PROJECT}}` | `{{SOURCE}}` |

- Before `upload`, the new release has no folder, so steps 1–3 use
  `{{ROOT}}`.
- `switch` commands run in the new release, which is not live yet.
  `current` points at it only after the action.
- `rollback` and `failed` use `{{ROOT}}`, because the release folder they
  would use may be the broken one, or gone.

### A deploy

```
preflight → build → upload → setup → backup
  → switch → restart → nginx → health → done

a failure in switch, restart, nginx or health:  → rollback → failed
a failure in any other step:                    → failed
```

### Runners

A command runs with `sh` unless it names another runner. Name one with a
single key: the runner, then the code.

```yaml
steps:
  setup:
    server:
      - npm ci --omit=dev                        # sh, the default
      - bash: set -o pipefail; npm run migrate | tee migrate.log
      - node: |
          const pkg = require("./package.json");
          if (!pkg.version) process.exit(1);
      - python3: import sys; assert sys.version_info >= (3, 11)
```

| Runner | How shipdeck runs the code |
| --- | --- |
| `sh` (default) | `sh -c '<code>'` |
| `bash` | `bash -c '<code>'` |
| `zsh` | `zsh -c '<code>'` |
| `node` | `node -e '<code>'` |
| `python3` | `python3 -c '<code>'` |

- The code goes to the runner as it is. Shipdeck does not change it, apart
  from placeholders.
- A runner command works like any other: same folder, same placeholders, same
  environment variables. A non-zero exit fails the step.
- A runner outside this list is an error. Adding one means adding it to
  shipdeck, with the flag that runs code from a string.

**Preflight checks every runner.** Shipdeck reads the runner names from the
config. It does not look inside your commands. Then, before anything runs:

- each runner used in a `local` command must be on your PC;
- each runner used in a `server` command must be on the server, checked along
  with `preflight.requires`.

A missing runner stops the deploy at preflight. The error names the runner,
the step and the app that use it. A program called *inside* a command, such as
`npm` in an `sh` command, is not found this way; list it in
`preflight.requires`.

## Backups and rollback

```yaml
release:
  steps:
    backup:
      keep: { open: 5, compressed: 15 }
      include: [generated]
      exclude: [packages/big-fixtures]
apps:
  api:
    steps:
      backup:
        include: [dist/cache]
```

`keep` says how many old releases stay on the server:

- **open**: releases kept unpacked, counting the live one. An open release
  is untouched since its own setup, so a rollback to one runs no setup.
- **compressed**: older releases kept only as `backups/<id>.tar.zst`. `0`
  keeps none.

**A backup always holds what was uploaded**: every file from `files`, plus
`.shipdeck/`. There is no switch to turn this off. Two lists change it:

- `include` adds paths that `setup` made on the server and cannot make
  again. Default: none. What `setup` can make again, such as `node_modules`,
  stays out.
- `exclude` leaves out uploaded paths, such as big files you can do without
  on a rollback. Default: none.

In `release`, both paths are in the release root. In an app, they are in the
app folder. Shared folders are never in a backup.

**A rollback to a backup acts like a fresh upload.** Shipdeck unpacks the
backup, links shared folders, and runs `setup.server` again, because the
backup does not hold what setup made. See [Rollback](#rollback).

## Rollback

A rollback puts an older release back. It happens two ways:

- **Automatic**, when `switch`, `restart`, `nginx` or `health` fails. A failure
  before `switch` needs no rollback: the old release never stopped serving,
  and the new release folder is removed.
- **By hand**, with `shipdeck rollback <deploy> <release>` or from the TUI.

### Settings

They belong to the whole release, so they live in `release.steps.rollback`:

```yaml
release:
  steps:
    rollback:
      mode: ask            # ask | auto | off. Default: ask
      ask_secs: 30         # with ask: roll back on its own after this. Default: 30
      keep_failed: false   # keep the failed release folder to inspect it. Default: false
      server: ./notify-ops.sh "rolled back to $SHIPDECK_PREVIOUS"
      local: ./notify.sh "shop rolled back"
```

| Setting | Meaning |
| --- | --- |
| `mode: ask` | Ask whether to roll back. With no answer within `ask_secs`, roll back. |
| `mode: auto` | Roll back at once. |
| `mode: off` | Never roll back on its own. The failed release stays live; roll back by hand. |
| `ask_secs` | How long `ask` waits. Today this is a fixed 30 seconds. |
| `keep_failed` | `false` removes the failed release after a rollback, as today. `true` keeps it in `releases/` to inspect. |

`mode` and `ask_secs` apply only to an automatic rollback. A rollback by hand
always runs.

### What a rollback runs

Like every step, `rollback` runs your commands first, then its action:

```
1. your commands:  rollback.server → rollback.local
2. the action:     [from a backup only: unpack → link shared → setup.server]
                   → switch back → restart → nginx → health → done
3. then, after an automatic rollback:  failed
```

Which commands run in a rollback:

| Commands | In a rollback |
| --- | --- |
| `rollback.server`, `rollback.local` | Yes, first, before the action. |
| `setup.server` | Only when the release comes back from a backup. An open release keeps what its setup made. |
| `health.server` | Yes, with the restored release's `health.url`, retried as in a deploy. |
| `done.server`, `done.local` | Yes, after the restored release is healthy. |
| `failed.server`, `failed.local` | Yes, last, after an automatic rollback. |
| `setup.local`, `health.local` | No. |
| `preflight`, `build`, `upload`, `backup`, `switch`, `restart`, `nginx` commands | No. |

Where the commands come from:

- `setup.server`, `health.server` and `done` belong to the restored release.
  They come from its `.shipdeck/settings.json`, so they match its files.
- `rollback` and `failed` come from the deploy file shipdeck was started
  with.

When `rollback.server` runs, `current` still points at the release being
replaced. `SHIPDECK_PREVIOUS` names the release being put back.

Other rules:

- A rollback checks the restored release with its own health checks. Today
  only the process state is checked.
- When nothing was live before, there is nothing to roll back to. The failed
  release stays, and the deploy ends as failed.
- A rollback by hand runs `failed` only if the rollback itself fails.
- If a rollback fails, the deploy ends as "rollback failed", and `failed`
  still runs.

Commands in `rollback` and `failed` get `SHIPDECK_STEP` (the step that
failed), `SHIPDECK_ERROR` and `SHIPDECK_PREVIOUS`.

## Apps

An app is a process (`process:`) or a static site (`static: true`). It is
exactly one of the two.

### A process

```yaml
process:
  name: shop-api        # default: <name>-<app>
  start: node dist/main.js
  env: { NODE_ENV: production }
  env_file: .env
  systemd:              # the manager
    user: deploy
    options:
      Service: { Restart: on-failure, MemoryMax: 1G }
```

**One manager per app.** The manager is the block you write: exactly one of
`systemd:`, `pm2:` or `oxmgr:`. None, or two, is an error. There is no
default manager; `pm2: {}` uses PM2 with no extra properties.

| Manager | Its block |
| --- | --- |
| `systemd` | `user`, then `options` or `template` |
| `pm2` | `options` or `ecosystem` |
| `oxmgr` | `options` |

**`options` are the manager's own properties**, written in its own format:

```yaml
systemd:
  options:                    # by unit-file section, as systemd names them
    Unit: { After: [network-online.target, postgresql.service] }
    Service: { Restart: on-failure, LimitNOFILE: 1048576, MemoryMax: 1G }
    Install: { WantedBy: multi-user.target }
pm2:
  options: { instances: 2, exec_mode: cluster, max_memory_restart: 300M }
oxmgr:
  options: { restart_policy: on_failure }
```

- **systemd:** each property goes into the section you name. It replaces the
  built-in unit's line of the same name; otherwise it is added. A list writes
  one line per item.
- **Shipdeck's own properties are an error in `options`.** They come from
  other keys: the command from `start`, the folder from the app folder, the
  environment from `env` and `env_file`, the name from `name`, and the systemd
  user from `user`. For systemd that is `ExecStart`, `WorkingDirectory`,
  `User`, `Environment` and `EnvironmentFile`. For PM2 it is `name`, `cwd`,
  `script`, `args`, `interpreter` and `env`.
- **A file of your own instead.** `systemd.template` is a whole unit file;
  `pm2.ecosystem` is a whole PM2 file. Each one replaces `options`, so using
  both is an error. With `pm2.ecosystem`, `start` is an error too.

- The process runs in `<dir>/current/apps/<app>`, through the `current` link,
  so a restart after the switch runs the new release. Relative paths in
  `start` and `env_file` begin there: `{{APP_CURRENT}}`. Default `start`:
  the binary in the app folder, `./<file name>` or `./<to>`.
- `systemd.template` is a path on your PC. You write it; shipdeck never looks
  for a template by itself. Without it, shipdeck uses its built-in unit.
- `pm2.ecosystem` is a path in the app folder, since PM2 reads it on the
  server.

### Planned: systemd instances

Not built yet. PM2 already runs several copies of an app through
`pm2.options` (`instances`, `exec_mode: cluster`). systemd will get the same
through template units: one unit file, `<process>@.service`, started as
`<process>@1`, `<process>@2`, and so on.

The planned shape:

```yaml
systemd:
  instances: 3          # starts <process>@1 to <process>@3
```

Reserved now, so the feature fits later without breaking a config. The code
enforces each rule from the first release of this format:

| Reserved | Rule now |
| --- | --- |
| `systemd.instances` | An error: "planned, not supported yet". |
| `{{INSTANCE}}` | An error. It will hold the copy's number, 1 to n. |
| `@` in a process name | An error. systemd uses `@` for instances. |
| `nginx.proxy_pass` | One URL now. It will also accept a list, one URL per copy. |

Still to decide before building it:

- How each copy gets its own port, or whether copies share one port.
- How `health` checks every copy.
- How `systemd.template` works with instances. It must then be a template
  unit.
- The sudo rules in `scripts/server-setup.sh` for `<process>@` units.
- oxmgr reads an [oxfile](https://github.com/Vladimir-Urik/OxMgr) and applies
  it with `oxmgr apply`.

### A static site

```yaml
apps:
  admin:
    source: apps/admin
    static: true
    files: [apps/admin/dist]
    nginx:
      root: dist                 # nginx serves {{APP_CURRENT}}/dist
      index: index.html
      domains: [admin.example.com]
```

`static: true` means nothing runs on the server for this app. Shipdeck ships
the files and switches `current`; it starts nothing.

- With `nginx:`, shipdeck serves the files through its managed nginx config.
  `nginx.root` and `nginx.index` are required. `root` is a folder in the app
  folder, and it must be in `files`. `index` is the file nginx serves for a
  folder, such as `index.html`.
- Without `nginx:`, serving the files is up to you, for example with your own
  web server pointed at `{{APP_CURRENT}}/dist`.
- Not allowed, each an error: `process`, `binary`, `shared`, `steps.setup`,
  and `steps.backup.include`. Nothing runs on the server, and nothing is made
  there.
- Everything else is allowed as in any app, including every other step's
  commands. `restart` has no action for a static app; its commands still run.

A deploy of a static app has no setup and no restart. The switch makes the new
files live. The `nginx` part runs only when the app has `nginx:`:

```
preflight → build → upload → backup → switch → nginx → health → done
```

### nginx and domains

`nginx:` is optional for every app:

- **With it**, shipdeck writes and manages the app's nginx config, checks it,
  and reloads nginx.
- **Without it**, shipdeck does not touch nginx. A process app is only
  started; a static app's files are only shipped.

```yaml
nginx:
  domains: [api.example.com]
  tls: { provider: letsencrypt, email: ops@example.com }   # or: none
  proxy_pass: http://127.0.0.1:3000   # process apps only
  root: dist                       # static apps only, in the app folder
  index: index.html                # static apps only
  headers: { X-Frame-Options: DENY }
  locations:
    /uploads/: { expires: 30d }
  raw: |
    client_max_body_size 20m;
```

- When `nginx:` is present, a process app must set `proxy_pass`. A static
  app must set `root` and `index`. Each is an error in the other kind of app.
  Shipdeck does not guess a port, a folder or a file.
- The file is checked with `nginx -t` before it is used, as the
  [nginx plan](nginx-config-management.md) requires.

### TLS certificates

A certificate belongs to a domain, not to a release. One certificate serves
every deploy after it. So getting one is not a step: the `nginx` step does it,
only when a certificate is missing.

For each app with `nginx.tls: { provider: letsencrypt, email }`:

```
certificate exists for every domain?
  yes → write the HTTPS config, check it, reload. Done.
  no  → write a plain-HTTP config, check it, reload,
        get the certificate (Let's Encrypt checks the domain over HTTP),
        write the HTTPS config, check it, reload.
```

- **First deploy, or a domain added later:** shipdeck gets the certificate.
  The domain's DNS must already point at the server.
- **Every other deploy:** nothing is issued.
- **Renewal** is not a deploy's job. A timer on the server renews
  certificates. `scripts/server-setup.sh` installs the certificate tool and
  the timer; a deploy never installs software.
- **Rollback** never touches certificates. They are server state, like
  `shared/`, and are never in a release or a backup.
- **If getting a certificate fails**, the app keeps its plain-HTTP config.
  What the deploy does then is open question 8.

`tls: none` serves plain HTTP only.

## Placeholders

A `{{NAME}}` placeholder works in every string of a deploy file: paths,
commands, `env` values, `nginx` settings and `health.url`. Shipdeck replaces it
before it uses the string. Your systemd template uses the same names.

| Placeholder | Value | Where |
| --- | --- | --- |
| `{{ROOT}}` | `/opt/shop`, the server `dir` | everywhere |
| `{{CURRENT}}` | `/opt/shop/current` | everywhere |
| `{{SHARED}}` | `/opt/shop/shared` | everywhere |
| `{{BACKUPS}}` | `/opt/shop/backups` | everywhere |
| `{{RELEASE}}` | `/opt/shop/releases/<id>` | from `build` on |
| `{{RELEASE_ID}}` | `v1.4.0-ab12cd3-20261110.120000` | from `build` on |
| `{{VERSION}}` | `1.4.0` | from `build` on |
| `{{NAME}}` | `shop` | everywhere |
| `{{HOST}}` | `prod` | everywhere |
| `{{PROJECT}}` | the project folder on your PC | everywhere |
| `{{CONFIG}}` | the deploy file's folder on your PC; the root file's folder for a deploy written in it | everywhere |
| `{{APP}}` | `api` | inside an app |
| `{{APP_CURRENT}}` | `/opt/shop/current/apps/api` | inside an app |
| `{{APP_RELEASE}}` | `/opt/shop/releases/<id>/apps/api` | inside an app, from `build` on |
| `{{APP_SHARED}}` | `/opt/shop/shared/apps/api` | inside an app |
| `{{SOURCE}}` | the app's source directory on your PC | inside an app |
| `{{PROCESS}}` | `shop-api` | inside a process app |

Rules:

- A placeholder is `{{`, a name in capital letters, digits and `_`, then
  `}}`. Spaces inside are allowed: `{{ ROOT }}` is `{{ROOT}}`. Other text
  with braces, such as JavaScript or Python code, is left alone.
- An unknown name is an error that lists the known ones.
- Shipdeck replaces the text as it is and adds no quotes. A path on your PC
  may hold spaces, so quote it in a shell command: `cd "{{PROJECT}}"`.
- A name used where it has no value is an error. For example, `{{APP}}` in
  `release:`, or `{{RELEASE}}` in `preflight`: the release is named at the
  start of `build`.
- Shipdeck checks every placeholder when it reads the file, before anything
  runs.
- YAML reads a value that starts with `{` as a map. Quote such a value, and
  any value inside `{ }` or `[ ]`: `path: "{{CONFIG}}/x"`.

**Environment variables.** A script file can't see placeholders, so every
command also gets each one as `SHIPDECK_<NAME>`: `{{CURRENT}}` is
`$SHIPDECK_CURRENT`, `{{APP_SHARED}}` is `$SHIPDECK_APP_SHARED`. Four values
change while a deploy runs, so they exist only as environment variables:

| Variable | Value |
| --- | --- |
| `SHIPDECK_ACTION` | `deploy` or `rollback` |
| `SHIPDECK_STEP` | the step that runs, or that failed |
| `SHIPDECK_PREVIOUS` | the release that was live before |
| `SHIPDECK_ERROR` | the error, in `failed` |

## `shipdeck tree`

Prints every path the next release will hold. It reads local files only and
never connects to the server. It uses the same code as `deploy`, so both
always agree.

```
shipdeck tree <deploy name or file> [--app <name>] [--depth <n>] [--build] [--json]
```

| Option | Meaning |
| --- | --- |
| `--app <name>` | Only that app's folder. |
| `--depth <n>` | Fold folders deeper than `n` into `name/ (<count> files)`. Default: no folding. |
| `--build` | Run the `build` steps first, so build output is there. |
| `--json` | One object per path: `path`, `kind`, `from`, `local`, `mode`, `size`. |

The right column says where each path comes from: `release`, `app <name>`,
`generated` or `link`. Output for universabile, with `--depth 3`:

```
$ shipdeck tree production --depth 3
/opt/universabile/releases/<id>   (current after the switch)
.
├── .shipdeck/                                     generated
│   ├── release.json
│   ├── settings.json
│   ├── universabile-server.pm2.json
│   └── universabile-website.pm2.json
├── apps/
│   ├── admin/
│   │   ├── dist/ (<count> files)                  app admin
│   │   └── package.json                           release
│   ├── server/
│   │   ├── .env                                   app server
│   │   ├── dist/ (<count> files)                  app server
│   │   └── package.json                           release
│   ├── vendor/
│   │   ├── dist/ (<count> files)                  app vendor
│   │   └── package.json                           release
│   └── website/
│       ├── .next/ (<count> files)                 app website
│       ├── cache -> /opt/universabile/shared/apps/website/cache   link
│       └── package.json                           release
├── package.json                                   release
├── packages/ (<count> files)                      release
├── pnpm-lock.yaml                                 release
└── pnpm-workspace.yaml                            release

<count> files, <size>.
```

When a pattern matches nothing, `tree` prints the tree it can, then the
missing patterns, and exits with 1.

## What changes from today

| Today | Format 2 |
| --- | --- |
| A deploy root scanned for subfolders with `shipdeck.yaml` | One root file (`root: true`) in the project folder, or `-c <file>` |
| A deploy root as a command argument | A deploy name, or a deploy file ending in `.yaml` or `.yml` |
| The format 2 draft's deploy-file flag | Removed: name the deploy file as the argument |
| No way to run from another folder | `--cwd` (`-C`) |
| A deploy is a folder | A deploy is one file, anywhere |
| `shipdeck.yml` also accepted | `shipdeck.yaml` or `.shipdeck` only |
| App files (`apps: {admin: _admin.yaml}`) | Apps inline in the deploy file |
| `service.template` found next to `shipdeck.yaml` | `process.systemd.template`, a path you write |
| `pipeline.pack.files.include.deploy` | A `files` entry with a `./` path and `to:` |
| `project` (top) | The project folder: where you run shipdeck |
| `project` (app) | `apps.<app>.source` |
| `ssh`, `dir` | `server.ssh`, `server.dir` |
| `service`, `process_manager`, `start` | `apps.<app>.process` |
| `manager: systemd`, `process_manager.name` | the block itself: `systemd:`, `pm2:` or `oxmgr:` |
| `service.run_as` | `systemd.user` |
| A unit template was the only way to change the unit | `systemd.options`, by section |
| PM2 `options` silently replace shipdeck's `name` and `cwd` | An error |
| `static.root` (top) | `apps.<app>.static: true` and `apps.<app>.nginx.root` |
| `pipeline` | `release.steps` and `apps.<app>.steps` |
| `pipeline.pack.files.include.project` | `files` |
| `pipeline.pack.files.exclude.*` | `exclude` |
| `pipeline.pack.binary` | `apps.<app>.binary` |
| `run: {local, server}` | `local`, `server` on the step itself |
| `setup.check.server` | the end of `setup.server` |
| `health.check.*` | `health.local`, `health.server` |
| top `pipeline` copied into apps | nothing is copied |
| `release.json`, unit files at the release root | `.shipdeck/` |
| `shipdeck-monorepo.json` | `.shipdeck/settings.json` |
| `SHIPDECK_APP_DIR`, `SHIPDECK_FOLDER` | `{{ROOT}}`, `{{CONFIG}}`, and `$SHIPDECK_ROOT`, `$SHIPDECK_CONFIG` |
| `SHIPDECK_RELEASE` (the name) | `{{RELEASE_ID}}`; `{{RELEASE}}` is now the folder |
| Unit template `{{dir}}`, `{{workdir}}` and others | The placeholders above |
| `pipeline.backup.keep` | `release.steps.backup.keep` |
| `failed.rollback` | `rollback.mode`, which adds `off` |
| A fixed 30-second rollback question | `rollback.ask_secs` |
| A rollback checks only the process state | It runs the restored release's health checks |
| Only some steps take commands, some after shipdeck's work | Every step: your commands, then shipdeck's action |
| A rollback by hand runs `done` | Every rollback runs `rollback`, then the restored release's `setup.server` (from a backup), `health.server` and `done` |
| `pipeline.backup.files.include`, `.exclude` | `steps.backup.include`, `.exclude` |
| `pipeline.backup.files.deployed` | Removed: a backup always holds what was uploaded |
| Rollback to a backup: unpack, set up, switch | Every step after upload, as for a fresh upload |

Server setup: the sudo rule in `scripts/server-setup.sh` installs
`current/<service>.service`. It becomes `current/.shipdeck/<service>.service`,
so each server runs the setup script once more.

## Open questions

1. **`.shipdeck` without `.yaml`.** Editors will not show it as YAML unless
   told to. Keep `.shipdeck`, or use `.shipdeck.yaml`?
2. **App order.** Name order needs nothing new. The order written in the file
   needs an ordered map, which may need a new dependency.
3. **nginx fields.** The nginx plan leaves the field list open. Is the shape
   above right, and how do `locations` and `raw` combine?
4. **oxmgr.** One oxfile for all oxmgr apps, or one per app? This depends on
   how `oxmgr apply` treats apps that are not in the file. Check
   [the oxfile spec](https://github.com/Vladimir-Urik/OxMgr/blob/master/docs/OXFILE.md)
   before building.
5. **Single-app process name.** The default `<name>-<app>` gives `web-web`
   for a single app named like its deploy. Keep the rule, or use `name` alone
   when there is one app?
6. **Certificate tool.** Which ACME client runs on the server: certbot,
   acme.sh or lego? It is a new server dependency, so it needs approval.
7. **Own certificates.** Should `tls` also accept files you already have,
   such as `{ cert: /etc/ssl/x.pem, key: /etc/ssl/x.key }`?
8. **Certificate failure.** The app itself is fine, and a rollback would not
   fix the certificate. Should the deploy finish with a warning and serve
   plain HTTP, or fail and follow `rollback.mode`?
