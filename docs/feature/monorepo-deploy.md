# Monorepo deployment

Deploy several apps from one repository as one release. CLI commands, the TUI,
and RPC use the same engine pipeline. Single-app YAML remains supported.
Nginx and oxmgr are future work.

## Configuration

Use `name`, `project`, `ssh`, `dir`, `process_manager`, `pipeline`, and `apps`.
A single-app deployment uses the same top-level `dir`, then `service` or
`static` instead of `apps`.

```yaml
name: product
project: ../..
ssh: {host: prod, user: deploy}
dir: /opt/product
process_manager: {name: systemd}
pipeline:
  build: {version_from: VERSION, run: {local: ./build-all.sh}}
  backup: {keep: {open: 3, compressed: 15}}
apps:
  api: apps/api.yaml
  web:
    project: web
    start: node server.js
    process_manager: {name: pm2}
    pipeline:
      pack: {files: {include: {project: [server.js, package.json]}}}
```

App files are relative to the main deploy file. They must stay inside its
folder, including after resolving symbolic links. Each app accepts `project`,
`start`, `env`, `env_file`, `shared`, `process_manager`, and `pipeline`.
App projects are relative to the repository's `project` folder and must stay
inside it. Process names are `<name>-<app>`. Custom PM2 names must be unique.

An app's `process_manager` replaces the whole global section. Other app pipeline
fields replace matching global fields. Objects merge by field; lists replace
lists. Commands have their own scopes: global commands run once, followed by
explicit app commands in app-name order. Inherited commands do not run again.
The global version file names the shared release. The global backup retention
and failure rollback policy apply to the whole release.

## Commands and working folders

| Step | Global commands | App commands |
| --- | --- | --- |
| Preflight | `run.server` in `dir`, then `run.local` in the repository. | Same order; server commands use `dir`, local commands use the app project. |
| Build | `run.local` in the repository. | `run.local` in the app project. |
| Setup | `run.server`, then `check.server`, in the new release root. | Same order in `releases/<id>/apps/<app>`. |
| Health | Checks after all process probes pass; server commands in `current`, then local commands in the repository. | Explicit checks in `current/apps/<app>` and the app project. |
| Done | Retention, then `run.server` in `current`, then `run.local`. | Commands in the app's live folder or project. |
| Failed | Local commands after recovery, including connection failures. | Explicit local commands in the app project. |

Health commands retry until the global health timeout. Each app uses its resolved
health URL and process timeout. Done failures produce a warning and keep the
healthy release live. Failed-hook errors produce a warning and preserve the
original failure.

Commands receive the existing `SHIPDECK_*` variables. App commands also receive
`SHIPDECK_APP` and `SHIPDECK_SERVICE`. `SHIPDECK_STEP` in failure commands names
the failed step. Server app commands load the app environment and environment
file. Preflight reads that file from the current app release if it exists.

## Release and backup behavior

One archive contains `apps/<app>/`, manager files, `release.json`, and
`shipdeck-monorepo.json`. The last file records resolved app settings for rollback.
A content fingerprint includes files, resolved settings, and stable manager
files. Unchanged deployment asks before continuing; the CLI needs
`--allow-unchanged`. File reuse and unpack verification work across apps.

Each app's binary is checked against the server architecture and glibc. Required
tools and permissions are checked before upload. HTTP checks require curl or
wget. A release name already on the server is refused before upload.

Shared folders live in `shared/apps/<app>/<path>`. Every release links to them.
Backup file rules apply inside each app folder. Manager files and saved settings
are always kept. Shared data stays outside backups. Global retention applies
once to the shared release.

After setup and backup, Shipdeck retires removed or replaced processes, switches
`current` once, installs all systemd units, restarts every app, and checks them.
Stopping or changing a systemd process requires passwordless stop and disable
permissions as well as the existing deployment permissions. The server setup
script includes them.

## Rollback and recovery

Manual rollback reads the target release's saved settings. A compressed backup
is unpacked, linked to shared data, and set up again before the switch. Local
source folders are not needed unless a saved local hook uses them.

Automatic recovery reads the previous release's saved settings. Both rollback
paths handle added and removed apps, PM2 name changes, and manager changes.
Permissions for both sides are checked before any process is changed. Removed
systemd processes are stopped and disabled. Removed PM2 processes are deleted
and the PM2 list is saved. Every restored app gets health checks.

Before a switch, a failure removes the staged release and keeps the live one.
After a switch, the global rollback policy decides whether to restore the old
release. When there was no previous release, the failed first release remains
available for inspection. External changes, including database migrations, are
not undone. Shared data is preserved.

## Inspection and testing

Status and service controls cover all live apps using the live release's saved
settings. Combined logs label each app. Release files and compressed backups can
be inspected through the same CLI/TUI/RPC paths as single-app releases.
Release inspection stays available when live process metadata cannot be read.

See the [SSH test guide](../code/monorepo-ssh-tests.md) for the Podman suite.
