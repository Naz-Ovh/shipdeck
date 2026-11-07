# Handoff

Current checkpoint: 2026-11-07.

Two sessions worked in parallel. This branch, `worktree-agent-p3a1-p3a2`, holds
the agent. `main` holds the monorepo work below. They merge later. Nothing on
this branch is committed yet; review it, then commit before merging.

## Agent (this worktree)

### What landed

`p3a1` and `p3a2`, the first half of the server agent:

- `crates/agent/`: package `shipdeck-agent`, its own binary and library.
  `serve(read, write)` uses the protocol crate's `Connection`. It answers
  `agent/hello`, `agent/ping` and `agent/shutdown`, then exits on shutdown or
  end of input. `--version` is what the engine probes.
- `crates/protocol/src/message/agent.rs`: the four agent message types and
  `PROTOCOL = 1`. They are in `protocol.schema.json` too.
- `crates/engine/src/agent/`: `binary.rs` picks gnu or musl and builds a fixed
  install script. The **server** downloads the archive over HTTPS, checks its
  `.sha256`, and unpacks it to `~/.shipdeck/bin/shipdeck-agent-<version>` with
  mode `0700`. `mod.rs` probes `--version`, falls back to
  `${TMPDIR:-/tmp}/.shipdeck-<uid>` on a `noexec` or unwritable home, starts it
  over an SSH channel, and says hello. Nothing is uploaded through SSH.
- `ssh::Session` holds the agent and starts it after login. A failure prints to
  the engine's stderr and never blocks the SSH deploy path.
- No new dependency. The server needs `curl` or `wget`, `sha256sum`, `awk`,
  `tar`, `find`, and outbound HTTPS.

### Decided

- The gnu build is first. Servers on glibc 2.31 (Ubuntu 20.04) or newer get it;
  Alpine, musl and older glibc get the static musl build. The agent package
  sets `min-glibc-version = 2.31`.
- The agent is installed and started on every connection, not lazily, as p3a1
  says. Plain deploys keep using the SSH shell path (p3d2, recommended).
- The launch command runs with no login-shell prelude: a startup file that
  prints would corrupt the pipe. Agent stderr goes to `agent.log` beside the
  binary.
- The remote binary's name carries the version, so an old one is never reused.

### Next for the agent

1. `p3a3`: move file work (unpack, verify, link, delete) into the agent.
2. The release carrying this code must include the four `shipdeck-agent-*`
   archives. v0.7.1 does not, so a download of the current tag 404s. Point
   `SHIPDECK_AGENT_BASE_URL` at a folder with the archive and its `.sha256`
   until then.
3. `dist generate` was not run: cargo-dist is not installed here. Run it before
   the release, or let the release workflow's check report the diff.
4. Still open: p3a4, p3b1, p3c1, p3c2, p3d1.

### Agent gotchas

- `~/.shipdeck/bin` may be `noexec`; the tmp fallback is deliberate.
- The agent speaks on stdout only. Diagnostics go to stderr.
- The install script is the only place that runs a shell for the agent. It takes
  no user text: the URL, target and path come from fixed code and the home
  folder, all quoted.
- `cargo run -p shipdeck` does not build the agent. Build it with `cargo build
  -p shipdeck-agent` and serve the archive from `SHIPDECK_AGENT_BASE_URL`.
- The agent's `main` calls `std::process::exit(0)` after `serve`, for the same
  stdin-thread reason the engine does.

## Monorepo deployment (main worktree)

The owner requested a Git checkpoint and a list of work still needed for
monorepo deployment. The feature is incomplete. It must not be described as
working because the component tests pass.

The agreed plan is recorded by commit `4073085` and
[docs/feature/monorepo-deploy.md](../docs/feature/monorepo-deploy.md).

### What exists

- Monorepo config parsing, inline and external app configs, and path validation.
- Global process-manager defaults. An app override replaces the whole section.
- Pipeline field overrides and resolution of app project paths.
- Typed single-app and monorepo loading. Discovery stores them separately.
- Shared bundle assembly with per-app folders and systemd or PM2 process files.
- Release metadata, metadata reading, and per-app backup file selection.
- Shared switch and restart functions used by the existing single-app flow.
- A preflight function for checking multiple app processes.
- Agent workflow rules in AGENTS.md. The .agents README is a documentation index.

### What prevents deployment

The RPC workspace loader explicitly returns "monorepo deployment is not available
yet" when discovery finds a monorepo. The new bundle, backup, and preflight
components are not connected to an executable monorepo pipeline.

### Remaining work

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

### Verification

At that checkpoint, these passed:

- cargo fmt --all -- --check
- cargo test --workspace: 300 tests across CLI, engine, protocol, and TUI.
- cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf
- git diff --check

Earlier in that session, a single-app PM2 deployment, a second release, and
manual rollback passed over SSH in Podman. That was not a monorepo test.

### Test environment and references

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
those are separate from the current scope.
