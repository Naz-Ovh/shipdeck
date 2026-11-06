# Handoff

Current checkpoint: 2026-11-07.

## Current work

The owner requested a Git checkpoint and a list of work still needed for
monorepo deployment. The feature is incomplete. It must not be described as
working because the component tests pass.

The agreed plan is recorded by commit `4073085` and
[docs/feature/monorepo-deploy.md](../docs/feature/monorepo-deploy.md).

## What exists

- Monorepo config parsing, inline and external app configs, and path validation.
- Global process-manager defaults. An app override replaces the whole section.
- Pipeline field overrides and resolution of app project paths.
- Typed single-app and monorepo loading. Discovery stores them separately.
- Shared bundle assembly with per-app folders and systemd or PM2 process files.
- Release metadata, metadata reading, and per-app backup file selection.
- Shared switch and restart functions used by the existing single-app flow.
- A preflight function for checking multiple app processes.
- Agent workflow rules in AGENTS.md. The .agents README is a documentation index.

## What prevents deployment

The RPC workspace loader explicitly returns "monorepo deployment is not available
yet" when discovery finds a monorepo. The new bundle, backup, and preflight
components are not connected to an executable monorepo pipeline.

## Remaining work

1. Connect discovered monorepos to the RPC workspace and operation dispatch.
   Extend CLI and TUI data handling without fabricating a single-app config.
2. Connect global and per-app preflight, build, setup, health, done, and failed
   commands. Define and document their execution order and working folders.
3. Connect shared release naming, binary checks, bundle assembly, fingerprints,
   unchanged detection, upload, verified unpacking, and shared-folder links.
4. Apply backup selection and retention to one shared release. Wire process
   installation, restart, and health checks for every app.
5. Implement manual and automatic rollback using the target release's saved
   settings. Handle apps added or removed, process-manager changes, partial
   failures, and compressed backups. Check permissions before changing live
   processes.
6. Complete release inspection, service controls, status, and logs for monorepos
   through the existing RPC and UI paths.
7. Publish a schema covering both deployment shapes. Update the CLI manual,
   website docs, feature docs, examples, and progress tracker to match reality.
8. Test full monorepo deployments and rollback over SSH in Podman. Cover mixed
   managers, external configs, app overrides, hook failures, shared data,
   unchanged content, backup restore, and single-app compatibility.

Nginx and oxmgr remain future work.

## Verification

At this checkpoint, these passed:

- cargo fmt --all -- --check
- cargo test --workspace: 300 tests across CLI, engine, protocol, and TUI.
- cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf
- git diff --check

Earlier in this conversation, a single-app PM2 deployment, a second release,
and manual rollback passed over SSH in Podman. That was not a monorepo test.

## Test environment and references

The temporary container is named `shipdeck-monorepo-ssh-test`. It was running
with systemd, SSH, PM2, and zstd. Its last SSH mapping was 127.0.0.1:38733.
Verify its current state before using it. The temporary test key was under
`/tmp/shipdeck-ssh-test/`. Ignored single-app test files are under
`.tmp/ssh-smoke/`.

Ignored code under `.tmp/monorepo-prototype/` is an abandoned, unintegrated
prototype. It is not proof of feature completeness. The owner withdrew the
request for separate single-app and monorepo engine folders. Existing engine
modules should be extended by responsibility.

## Earlier project context

Shipdeck is one CLI command. Its engine communicates with the CLI and TUI over
the protocol crate. The last earlier project work connected those clients to
the engine. Automatic shutdown safety and Windows behavior still had open work;
those are separate from the current monorepo scope.
