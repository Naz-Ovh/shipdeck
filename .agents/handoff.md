# Handoff

Current checkpoint: 2026-11-07.

## Current work

The owner requested complete monorepo deployment, including RPC, CLI, TUI,
shared releases, backups, saved-settings rollback, documentation, and real SSH
checks in Podman. That implementation is complete. The owner requested a checkpoint commit.
The referenced server-agent plan is separate from this monorepo work.
Nginx and oxmgr remain future work.

## Implementation

- RPC workspaces store the typed Deployment enum. Monorepos no longer reject
  workspace opening or impersonate single-app configs.
- Deploy and rollback dispatch through the existing CLI/TUI protocol paths.
- The shared pipeline lives in deploy/multiple*.rs, split into orchestration,
  commands, release transport, and recovery. deploy/apps.rs handles saved app
  settings and process handles. Existing single-app behavior remains supported.
- Global commands run once, then explicit app commands in app-name order.
  Pipeline settings inherit by field; command defaults are not repeated per app.
  Failed commands run locally even when SSH is unavailable.
- Shared release naming, per-app binary checks, stable fingerprints, file reuse,
  verified unpacking with a full-upload retry, and app shared links are wired.
- Release IDs already on the server are refused before staging. Cleanup tracks
  whether this operation staged or restored files, preventing deletion of an
  existing release after an early failure.
- Backups use per-app selection and one global retention policy. Saved settings,
  generated manager files, and custom PM2 ecosystem files are always preserved.
- One release switch installs all processes, restarts every app, and checks them.
- Manual and automatic rollback read saved settings. They handle apps added or
  removed, PM2 names, manager changes, partial activation, and compressed backups.
  Failed compressed restore setup removes its open folder and keeps the backup.
- Both sides of a process transition get permission checks before live changes.
  Removed systemd processes are stopped and disabled. PM2 processes are deleted
  and saved. server-setup.sh now includes systemctl disable permissions.
- Status and controls use live saved settings. Combined logs label each app.
  Release inspection does not depend on readable live process metadata.
- The published schema accepts both shapes. The CLI manual, website guide,
  examples/monorepo, feature guide, spec, and progress tracker are updated.
  Tracker p1d3 is built, awaiting the owner's own trial or a release.

## Verification

Passed:

- cargo fmt --all -- --check.
- cargo test --workspace: 302 tests, across CLI, engine, protocol, and TUI.
- cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf.
- Both published schema shapes, their descriptions, and examples are checked.
- git diff --check and server-setup.sh syntax checks.
- Monorepo example JavaScript and progress tracker JavaScript syntax checks.
- Documentation website production build. Existing CSS highlight warnings remain.
- A real single-app PM2 SSH deployment with the new RPC dispatch.
- tests/monorepo_ssh.py over SSH in Podman: mixed systemd/PM2, external app files,
  global/app hooks, environment files, unchanged detection, shared data,
  retention, compressed backup rollback, failed restore setup, setup failure,
  app additions/removals, manager changes, automatic recovery, missing service
  files during activation, done warnings, and custom PM2 ecosystem backups.
- RPC status, release list/read/contents, service stop/start/restart, combined
  logs, and log cancellation were tested while local and live app lists differ.

Final SSH suite log: .tmp/monorepo-e2e-release.log.
Suite sources and detailed command log:
/tmp/vscode-agent-codex-sandbox-H2LGk7/shipdeck-monorepo-e2e-vth54x9t/.
The final focused RPC check also covers missing live metadata; its log is
rpc-final.log in that same folder. Workspace results are in
.tmp/workspace-tests-final.log. Website results are in .tmp/website-build.log.

## Test environment

The disposable container shipdeck-monorepo-ssh-test was running with systemd,
SSH, Node.js, PM2, zstd, and curl at 127.0.0.1:38733. The SSH key is under
/tmp/shipdeck-ssh-test/. Verify these before reuse.

Rootless Podman cannot apply the built-in unit's mount namespace hardening.
The SSH suite uses a test-only custom service template; production hardening
is unchanged. A 15-second health timeout allows three stable probes under the
shared container's load. Test services, releases, and logs remain for inspection.

Cargo and Node.js require commands outside the restricted filesystem. The
installed Node.js is /home/bns/.nvm/versions/node/v26.12.1/bin/node. pnpm is not
on PATH; the site was built through node_modules/.bin/next with that Node PATH.
A stale .next cache referenced another checkout under /mnt/wsl. It was moved
reversibly to .tmp/website-next-before-monorepo-check, and a fresh build passed.
No project dependency was added. Curl was installed only in the test container.

## Earlier context

Ignored .tmp/monorepo-prototype is an abandoned prototype and is not used.
The owner withdrew a request for separate single-app and monorepo engine
folders; the new modules are divided by responsibility within the existing
engine. Automatic shutdown safety and broader Windows behavior remain separate
from this monorepo scope. The earlier agreed plan was recorded by commit 4073085.
