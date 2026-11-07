# Handoff

Current checkpoint: 2026-11-07.

Two efforts are merged on `main`: the server agent (`p3a1`, `p3a2`) and the
completed monorepo deployment. The agent notes are under `## Agent` below.

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
  mode `0700`. `mod.rs` probes `--version`, falls back to a fresh `mktemp -d`
  folder in tmp on a `noexec` or unwritable home, starts it over an SSH channel,
  and says hello. Nothing is uploaded through SSH.
- `ssh::Session` holds the agent and starts it after login. A failure prints to
  the engine's stderr and never blocks the SSH deploy path.
- No new dependency. The server needs `curl` or `wget`, `sha256sum`, `awk`,
  `tar`, `find`, `timeout`, and outbound HTTPS.

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

### Review fixes (2026-11-07)

`docs/review/server-agent-review.md` raised two P1s, two P2s and a layout rule
over two rounds. The review now starts with `(done)`. Fixed:

- The tmp fallback is a fresh `mktemp -d` folder, checked for a symlink, this
  user's ownership, and mode `0700`. Concurrent connects get different folders.
- Timeouts now cover the optional startup: 15 s per raw command, 45 s for the
  install (curl/wget also bounded), 15 s to open and hello, 90 s overall.
- The second review found that a channel close alone does not stop a non-PTY
  process. Limited commands now run under `timeout -k 3 <limit>` on the server,
  which signals the command's process group; the channel close is the backstop.
  The engine maps exit 124 or 143 to a timeout.
- Every unsupported request with an id, including a recognised `ask/*`, gets
  `METHOD_NOT_FOUND`. A process-level test runs the real binary.
- `crates/agent/src/lib.rs` and `crates/engine/src/agent/mod.rs` are now
  declarations and re-exports only. The code is in `serve.rs`, `session.rs`
  and `install.rs`.

Verified on the test container: forcing the home install to fail made the
secure tmp fallback run; a stalled download returned after 90 s with the
release list intact and no `curl` left behind. A real SSH check ran
`timeout -k 3 1 sh -c "sleep 3; printf done > marker"`: it returned 124 and
the marker was absent 4 s later. The agent install still completed through the
wrapper.

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

## Earlier context

Ignored .tmp/monorepo-prototype is an abandoned prototype and is not used.
The owner withdrew a request for separate single-app and monorepo engine
folders; the new modules are divided by responsibility within the existing
engine. Automatic shutdown safety and broader Windows behavior remain separate
from this monorepo scope. The earlier agreed plan was recorded by commit 4073085.

## Nginx research — 2026-11-07

The owner said the server-agent plan is being implemented by another LLM and
requested research into programmatic Nginx control. Findings and source links
are in docs/feature/nginx-control-research.md. This is research only; no agent
code, dependencies, server settings, agreed schema, or tracker state changed.

Verified from official sources: Open Source 1.31.5 introduced the Control API;
it requires --with-control-api and explicit -l startup configuration. It reads
loaded configuration and workers and requests full reloads, but does not write
site files. A restricted Unix socket is recommended. Command-line validation
and reload remain the proposed compatibility path for other installations.

The note proposes managed configuration generations, full validation, reload
verification, routing recovery tied to saved app settings, and a narrow root
boundary. p3d1 remains undecided. It compares the official Nginx ACME module
with Certbot webroot/DNS options; no certificate tool was selected. No runtime
Nginx tests were performed. The research note and this handoff are uncommitted.

## Isolated Pingora experiment — 2026-11-07

The owner asked whether Pingora has a large dependency tree and requested a
hands-on developer-experience test in an isolated folder. The experiment is
complete at /tmp/vscode-agent-codex-sandbox-H2LGk7/pingora-lab/.
Read its README.md for the report, src/main.rs for the prototype, and
measurements.json for machine-readable measurements. It has its own workspace,
Cargo.toml, Cargo.lock, feature variants, logs, and test harness. No Shipdeck
application code, dependency, or lockfile was changed.

Published Pingora 0.9.1 was tested with Rust 1.99.1 on Linux x86_64. Unique Linux
normal/build dependency package-version counts, excluding the experiment root:
142 for core, 167 for HTTP proxy, 176 for proxy plus OpenSSL, and 179 for load
balancing plus OpenSSL. The tested TLS graph has 167 distinct crate names and
12 Pingora crates. The cross-platform lockfile has 203 dependency packages.
The proxy includes pingora-cache without explicitly enabling the facade's cache
feature. The selected OpenSSL feature vendors OpenSSL; native zlib-ng needs CMake.

The initial debug build failed after 75.03 seconds because CMake was absent.
CMake 4.4.4 was installed only into the experiment's .tools Python environment.
The resumed debug build succeeded in 26.77 seconds. The fresh release build took
177.08 seconds. A small source-edit debug rebuild took 1.674 seconds; a no-change
build took 0.125 seconds. The stripped release binary is 14,237,648 bytes
(13.58 MiB). A post-test release process RSS sample was 14,908 KiB (14.56 MiB).
Combined debug/release/test/check artifacts occupy about 1.8 GiB. These are
machine-specific development measurements, not throughput or capacity claims.

The 164-line prototype uses ProxyHttp callbacks, loopback HTTP backends, HTTP
and TLS listeners, and a custom stdin control channel. Complete route snapshots
are parsed first and replaced under one short-held lock. No lock spans an await.
Functional checks passed in debug and release: two-host routing, case handling,
query forwarding, verified self-signed TLS, a 1,125,000-byte POST body checked by
hash, rejection of invalid updates without changing live routes, updates without
restarting the process, in-flight requests completing on their original backend,
route removal, and recovery after an unavailable backend returns 502.
Three route unit tests, formatting, and strict pedantic/performance Clippy passed.

Assessment: the dependency tree is substantial, but the basic Rust proxy API is
approachable and incremental rebuilds are quick after setup. A complete gateway
still needs certificate issuance/renewal, per-domain certificate selection,
static-file behavior, durable configuration, and a real management API. Those
were not implemented or tested. No Nginx/Caddy performance comparison was run.
The owner has not selected Pingora or authorized integration into Shipdeck.

## Agreed Nginx configuration plan — 2026-11-07

The owner chose an owned Rust parser and writer rather than adopting
nginx-lint-parser, Rowan, or Crossplane. Final plan:
docs/feature/nginx-config-management.md. The visual spec records this decision.
No implementation or tracker status changed. No application dependency changed.

The plan covers lossless source, structural editing, system configuration,
include graphs, unknown directives, native validation, and recoverable agent
transactions. Follow actual Nginx grammar and test against real Nginx. Do not
claim generic syntax support provides every module's semantics.

Dependency review evidence is in
/tmp/vscode-agent-codex-sandbox-H2LGk7/nginx-parser-deps/:
tree.txt, metadata.json, maintenance-crates.json, and maintenance-repos.json.
Parser 0.24.0 has 16 default transitive packages. The parser and Rowan are active;
countme is archived; text-size migrated into Rust Analyzer; memoffset is quiet.
This was a maintenance review, not a vulnerability audit. No library was adopted.

## Server-agent review — 2026-11-07

Nginx planning and research committed as a1ea865. Reviewed the other agent's
implementation from 3a045b7, merged by ecb1d3f. Full findings and verification:
docs/review/server-agent-review.md. The review is marked `(done)`; the fixes
are in the working tree.

Two P1 findings: unsafe existing temporary installation directory handling
(root fallback can trust another user's directory), and unbounded optional
agent download/startup blocking the normal SSH connection path. One P2 finding:
recognised ask/* requests are silently skipped instead of returning an RPC
method error. Local shell/process reproducers confirmed the specific behaviors;
no cross-user exploit or new Podman SSH suite was run.

Formatting, 319 workspace tests, strict Clippy, and the agent executable build
passed at review time. Review evidence:
/tmp/vscode-agent-codex-sandbox-H2LGk7/agent-review/. The original findings
were fixed; the second review below reopened one cleanup finding.

## Independent agent re-review — 2026-11-07

Reviewed the current uncommitted corrections. Formatting, all 329 workspace
tests, and strict pedantic/performance Clippy passed. The secure temporary
folder and unsupported Ask RPC fixes address the original findings. Startup
is now bounded for the caller and the SSH session remains usable.

The re-review reopened docs/review/server-agent-review.md with one P2 cleanup
finding. A real Session::exec_raw_within call over SSH in the existing Podman
container returned after about 344 ms for a 300 ms limit, but its remote command
still created a marker eight seconds later. Channel closure does not establish
remote process termination. The test removed its marker and closed the session.
Reproducer: /tmp/vscode-agent-codex-sandbox-H2LGk7/agent-recheck/src/main.rs.

The cleanup finding is fixed: limited commands run under
`timeout -k 3 <limit>` on the server, which signals the command's process group;
the channel close is only the backstop. A real SSH check returned 124 and left
no marker 4 s later. The review is `(done)` again. The fixes are in the working
tree and remain uncommitted.
