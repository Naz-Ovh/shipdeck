# Static site mode

Status: built on 2026-11-08. The owner has not tried it yet.
Tracker row: `p4a0`, before `p4a1`.

Implemented: `static:` in shipdeck.yaml and per monorepo app, the shared path
rule (`./` accepted, `/` and `..` refused), the release check for `static.root`,
no unit or restart, HTTP-only health, and static status in the screens.
Not implemented: nginx configuration, domains, HTTPS, SPA fallback, `shared`
folders for static apps, and an editable monorepo layout.

## Problem

Shipdeck assumes every app is a long-running process. The config needs
`service.start`, or `pipeline.pack.binary` to default it, and a deploy always
ends with unit install, restart and a process probe.

A static site has files and nothing to run. Today that fails at config check:

```
app admin: service.start is needed when there is no pipeline.pack.binary
```

Nginx management is still future work. This step ships static files without
running a fake process, and leaves the site to a web server the owner sets up
once.

## Decision

- `static:` is a peer of `service:`. Exactly one is required.
- In a monorepo, each app has `start` (process) or `static` (files).
- `static.root` is relative to the app's release folder. Packing stays
  explicit in `pipeline.pack.files`.
- Shipdeck does not write nginx configuration, domains or HTTPS in this step.
- A shared path rule accepts `./`, rejects a leading `/`, and rejects `..`.
- The monorepo layout stays `apps/<app>`. An editable layout is future work.

Not chosen:

- `service.manager: static`: mixes a process manager with no process. Many
  fields and agent messages assume systemd or PM2.
- Inferring static from a missing `start`: a typo in `start` would silently
  deploy files only.
- Managing nginx now: the nginx plan is large and separate
  ([nginx-config-management.md](./nginx-config-management.md)).

## Configuration

`dir` is top level for every deploy. A single deploy and a monorepo app both
use `dir` and never `service.dir` or `static.dir`.

One static site:

```yaml
name: Admin
project: apps/admin
dir: /var/www/admin
ssh: { host: prod, user: deploy }

static:
  root: dist

pipeline:
  build:
    run:
      local: pnpm build
  pack:
    files:
      include:
        project: [dist]
  health:
    url: https://admin.example.com/
```

A monorepo with one static and one process app:

```yaml
name: product
project: ../..
ssh: { host: prod, user: deploy }
dir: /opt/product

apps:
  admin:
    project: apps/admin
    static:
      root: dist
  api:
    project: apps/api
    start: node dist/main.js
```

| Field | Needed | Meaning |
| --- | --- | --- |
| `dir` | yes, top level | Server folder for `current`, `releases/`, `backups/` and `shared/`. Every deploy has one. |
| `static.root` | yes | Folder inside the release to serve. Relative, normalized, not `.`. |

Monorepo apps share the top-level `dir`; their release folder is
`apps/<app>`, so they need only `root`. A single deploy uses the same top-level
`dir`. Nothing under `service` or `static` names it.

Served path, once live:

| Deploy | Path |
| --- | --- |
| single | `<dir>/current/<root>/` |
| monorepo | `<dir>/current/apps/<app>/<root>/` |

`current` is the symlink the switch moves, so the path never changes between
releases. nginx points at that path once.

## Packing stays explicit

`root` says where to serve, not what to pack. Files can come from the project
or from the deploy folder, and a release can carry more than one folder. A
hidden "root means include from project" rule would guess the source wrong.

`root` must exist in the packed files. The pack step fails before upload when
it does not:

> `static.root` is `dist`, but the release has no `dist` folder. Add it to
> `pipeline.pack.files.include`.

## Path rule

One helper normalizes a path that must stay inside a base folder:

- Keep only plain folder names; drop `.` parts.
- Return an error for a leading `/`, a Windows prefix, or any `..`.
- Return `None` when nothing is left, for fields that require a target.

Use it for:

- `static.root`;
- app config files (`apps: admin: ./apps/admin.yaml` now works);
- the other fields that already behave this way (`env_file`, `shared`,
  `pm2.ecosystem`). No change there; the helper replaces the duplicated code.

`project` is exempt: it may climb with `..` on purpose (`../../../web`).

For app files, the existing check stays: canonicalize the result and confirm it
is still inside the deploy folder. The helper makes `./` and `dist` the same
path before any comparison.

## Validation

- One of `service` or `static`, never both and never neither.
- Per app: `start` or `static`, never both. `pipeline.pack.binary` is refused
  with `static`.
- `dir`: top level, absolute, no `..`, same character rules as before.
- `static.root`: normalized with the path rule; not empty, not `.`.
  The release root holds `release.json`, which can contain env values.
- `static.root` must be a folder in the packed contents, separate for each
  monorepo app.
- Process fields are refused on a static app: `start`, `process_manager`,
  `pm2`, `run_as`, `env`, `env_file`. A clear message says to use `service`, or
  to remove the field.
- `shared` on a static app is not supported in this step.

## Deploy behavior

| Step | Static site |
| --- | --- |
| Preflight | Same checks, minus the start program and PM2/systemd probes. Creates `dir` when missing. |
| Build, Pack | Same; adds the `root` check. |
| Upload, Unpack | Same. |
| Setup, Backup | Same. `setup.run.server` and `check.server` run in the release folder. |
| Switch | Moves `current`. Writes no unit, installs nothing. |
| Restart | Skipped: "nothing to restart". In a monorepo, skipped only when every app is static. |
| Health | HTTP checks only. No process state, no restart counter. `pipeline.health.url` is optional but recommended. |
| Done | Retention and hooks as today; the log names the served path. |
| Rollback | Moves `current` back. No restart. Health runs again. |

A monorepo release can mix static and process apps. Switch installs units for
the process apps only; restart and the process probe touch those apps only.

## Internals

- Config: `DeployConfig` has optional `service` and optional `static`.
  `StaticSite` holds `dir` and `root`. An app holds optional `static`.
- One target type in the engine, either a `Service` or a `StaticSite`, so the
  deploy code branches once instead of checking `Option`s everywhere.
- The agent protocol does not change. Only process targets send the service
  message.
- Remote: `install_unit`, `restart` and the process probe are process-only.
  Static health uses one HTTP probe with the existing curl/wget client check.
- Bundle: no unit file, no process file. The `root` check reads the contents
  that were just collected.
- CLI, TUI and status show `static` and the served path instead of a process
  state.

## Tests

- Parse: one of each; both; neither; `static: {root}`; app-level root; top-level `dir`.
- Path rule: `./dist`, `dist/./public`, `/dist`, `../dist`, `.`, empty.
- App file paths: `./apps/admin.yaml` works; `../admin.yaml` fails; a symlink
  that leaves the deploy folder fails.
- Root checks: missing from the release; a file instead of a folder;
  `root: .`; binary with static.
- Deploy: restart skipped; no unit file written; health uses the URL; rollback
  switches back; done names the path.
- Monorepo: one static and one process app in one release; only the process app
  gets a unit and a restart.
- SSH test in disposable Podman: nginx serves `current/<root>`, a switch
  changes the file it serves, and a rollback restores it.

## Verification

- `cargo test --workspace` passes, including new tests for static parsing,
  `./` paths, absent or conflicting targets, the release check, a static
  monorepo app, and static app files.
- `just schema` regenerated `docs/schema/deploy.schema.json` and the website
  copy. `shipdeck man configs` lists `dir`, `static` and `static.root`.
- Not run: an SSH deploy in Podman. This machine's rootless Podman could not
  start (`/run/user/1000/libpod` is read-only), so the full static deploy is
  untested end to end. The owner should try `examples/static` on a server.

## Not in this step

- nginx configuration, domains, HTTPS and DNS records.
- nginx settings for a site: headers, locations, a path override and a raw
  config block. Direction is saved in
  [nginx-config-management.md](./nginx-config-management.md#app-facing-nginx-settings).
- SPA fallback (`try_files ... /index.html`) and cache headers.
- Editable monorepo layout (`apps_dir` or similar). Today `apps/<app>` is
  fixed. The owner wants this later; `static.root` is relative, so it survives
  the change.
- `shared` folders for static sites.
- Automatic nginx-user permission checks. nginx needs traverse permission on
  every parent of `current`; release folders are created world-readable by
  default, so a folder under `/var/www` works.

## Open questions

- Tracker id: `p4a0` under Domains. Kept; change it if the owner prefers
  another phase.
- Health URL: optional. Default: no URL check; the deploy still reports the
  served path. Recommended for every site that has a domain.
- nginx reload after a switch: not needed for static files, because nginx reads
  the `current` symlink per request. Routing changes stay in the nginx phase.
- Editable layout shape: save for the future monorepo step. A single folder
  name (`apps_dir: apps`) is better than a path pattern language.
