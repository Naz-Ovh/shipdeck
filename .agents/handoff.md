# Handoff

## Config file renamed to shipdeck.yaml — 2026-11-09

Discovery, `init`, the manual, docs, examples and tests now use
`shipdeck.yaml` or `shipdeck.yml`. `deploy.yaml` and `deploy.yml` are no longer
read. The JSON Schema file keeps its name, `deploy.schema.json`. The docs page
moved to `/docs/reference/shipdeck-yaml`. Breaking change; the upgrading guide
has the `mv` step. `just verify` and the docs build passed.

## Nginx app settings direction — 2026-11-09

The owner wants a dedicated nginx block in `shipdeck.yaml` when nginx is built,
separate from `service` and `static`. It should cover response headers,
`location` blocks, a served path override, and raw nginx configuration as a
string block. Field name and exact shape are open. Saved in
`docs/feature/nginx-config-management.md#app-facing-nginx-settings`; tracker
row `p4a4` is planned. No code changed for this.

## Static site mode — 2026-11-08

Built `static:` as the second kind of deployment. A deploy (or a monorepo app)
is a `service` or a `static` site, never both. A static site has no unit, no
restart and no process probe. The release switches `current` as before, and
nginx serves `<dir>/current/<root>` (single) or
`<dir>/current/apps/<app>/<root>` (monorepo), so the path does not change.

### What changed

- **Config.** `DeployConfigFile` accepts `service` or `static`, and a required
  top-level `dir` for every deploy. `DeployConfig` stays a runtime type with one
  `Service`; a static site is a `Service` with `manager: static` and a
  `static_root`. `service.dir` and `static.dir` are gone; both are refused.
  Monorepo apps and single static deploys take `static: {root}`.
- **Paths.** One `inside_path` helper accepts `./dist`, `dist/./public`,
  rejects `/dist`, `..` and an empty result. It is used for `static.root` and
  for app config file paths, so `./apps/admin.yaml` now works.
- **Checks.** Exactly one target; no `pipeline.pack.binary`, `start`,
  `process_manager`, `env`, `env_file` or `shared` with a static app. The pack
  step stops when `static.root` is not in the release.
- **Deploy.** Restart is skipped for static; health is HTTP-only (and skips
  when no URL is set); install/restart/control/logs are no-ops or clear errors.
  Monorepo switch installs only process units and restarts only process apps.
- **UI and docs.** Static sites show `static` in the TUI, with controls and
  logs refused with a clear message. Schema, manual, website reference,
  monorepo guide, examples guide, `examples/static`, and tracker `p4a0` are
  updated.

### Verified

`just verify` passed after the `dir` move: formatting,
`cargo check --workspace --all-targets`, strict pedantic/performance Clippy,
and all workspace tests. `just schema` regenerated both schema copies, and the
docs website build passed. The CLI parses and starts a static deploy; the test
run stopped at the unreachable example host.

Not verified: a full static deploy over SSH. Rootless Podman cannot start in
this environment (`newuidmap: write to uid_map failed`), and no `sshd` is
installed. The owner should deploy `examples/static` to a real server with
nginx pointed at `/var/www/static/current/dist`.

## v0.8.1 release restart prepared — 2026-11-08

The owner authorized committing the runner fix, the Apache license removal,
and the server setup script move, then restarting the release. The old
v0.8.1 run (42494366128) is cancelled and no v0.8.1 GitHub release exists.
Remote main and the tag both pointed to b6ff59f before this update.

The workspace license and design/progress docs now say MIT. The setup script
lives at `scripts/server-setup.sh`; engine download URLs, test paths, the
manual, script example, and layout docs use its new location. Its executable
behavior is unchanged. The workspace version remains 0.8.1.

Fresh checks passed: formatting, workspace check with all targets, strict
pedantic/performance Clippy, all 395 Rust tests, six artifact gate tests,
Bash syntax, `dist generate --check`, and `git diff --check`. The release plan
selects native `macos-15` for Apple Silicon. The restart must use a new tag
push on the fixed commit, because a rerun keeps the old commit.

## Release CI macOS runner — 2026-11-08

The CI error `The configuration 'macos-14-arm64-us-default' is not supported`
comes from cargo-dist 0.33.0 selecting `macos-14` for `aarch64-apple-darwin`.
GitHub retired macOS 14 runners on November 2, 2026:
https://github.com/actions/runner-images/issues/13518.

Added an explicit `macos-15` runner in `dist-workspace.toml`. The generated
workflow reads its matrix from the plan, so no workflow edit is needed.

Verified with the pinned dist 0.33.0: `dist plan` before and after, and
`dist generate --check`. A comparison of both plans confirms that only the
Apple Silicon runner changes; hosts, targets, Linux containers, and other
runners stay unchanged. All six release artifact tests and `git diff --check`
passed. No Rust source changed, so Rust checks were not run. A hosted macOS
build was not run. No commit, tag, push, or release action was performed.
An existing failed release must use a commit containing this fix; rerunning
the old tag will still use the old configuration.

## Privileged agent install and version policy — 2026-11-08

Implemented the agreed policy in `docs/feature/agent-version-policy.md`. No
version was selected, committed, pushed, or published.

### What changed

- **Install and launch.** The agent installs at
  `/opt/shipdeck/agent/v<version>/agent`. Root owns the binary and every parent
  directory. The connection must be root or have password-free sudo. The agent
  starts through `sudo` with the full path, and the upload receiver uses the
  same identity.
- **Atomic install.** One lock covers the install. It uses `flock`, with an
  atomic directory lock and stale-owner detection when `flock` is absent.
  Downloads go to private root staging. The checksum, archive paths, entry
  types, and printed version are checked before a complete version directory
  is published in one rename. Older versions are removed after a successful
  publish.
- **Version rules.** Every connection lists valid installations. Same-major
  minor and patch versions are compatible. A newer agent major refuses the
  connection. A newer engine major clears older agents and installs its own
  release. A newer installed major blocks the connection even when a same-major
  install also exists. The protocol is now 3 and the schema was regenerated.
- **No bypass.** A failed install, version probe, or handshake ends the
  connection. Operations do not continue through an incompatible agent.
- **Execution identity.** `AgentService.user` carries the service user. PM2
  commands and logs run through `runuser` with that user's home, login `PATH`,
  `PM2_HOME`, and service environment. New project, release, backup, and shared
  paths are given back to the SSH user. Existing app and service ownership is
  preserved, and the path, archive, symlink, and mode checks stay in force.
- **State.** The agent uses `SHIPDECK_OPERATOR_HOME` for
  `~/.shipdeck/state.db`, so a root agent does not move event history to
  `/root`. Diagnostics go to `~/.shipdeck-agent.log`.
- **Development upload.** Root copies the user-uploaded binary into staging,
  checks the copied SHA-256 and version, and runs only the root-owned copy. A
  user-writable binary is never executed as root.

### Verified in Podman

Container `shipdeck-agent-work` at `127.0.0.1:44681`, user `deploy` and
`shipdeploy`, with password-free sudo. The agent archive was served inside the
container. Logs are in `.tmp/e2e/`.

- First install with no agent, then reconnection.
- Same-major `0.6.9` installed and used while the CLI was `0.7.1`.
- Newer `1.0.0` agent refused; both versions and required major shown.
- An invalid older version directory was removed by a later install.
- Two simultaneous first connections both succeeded; one version directory was
  published. The same two-connection test passed with `flock` hidden, exercising
  the atomic directory lock and stale-owner path.
- Interrupted download ended with exit 18 and no valid version directory.
- Incorrect checksum, wrong binary version, and a handshake with a wrong
  version each failed without operations.
- The 17.6 GiB container: `/opt/shipdeck`, its parents, and the binary were
  root-owned. `deploy` could not write, remove, or chmod the binary or its
  directory.
- Development upload installed a root-owned `0755` copy; no user staging
  directory remained.
- A user without sudo got the access error before `/opt/shipdeck/agent` was
  created.
- PM2 deploy and restart ran as `shipdeploy`; release files, backups, and shared
  data were owned by `shipdeploy`. A systemd deploy succeeded. A systemd setup
  command wrote its user as `shipdeploy` (uid 1002).
- `server-info`, `work`, and `events` worked without a terminal.
- Existing `~/.shipdeck/state.db` events replayed through the root agent.

The current tree cannot run a real CLI-major upgrade because it is `0.7.1`.
The `Replace` decision and its cleanup are covered by unit tests; the same
cleanup path was exercised by removing an invalid older directory.

### Checks

- `cargo fmt --all -- --check`: pass.
- `cargo check --workspace --all-targets`: pass.
- `cargo test --workspace`: 395 tests pass (38 CLI, 48 agent, 2 agent process,
  249 engine, 24 protocol, 34 TUI).
- Strict Clippy with pedantic and performance warnings: pass.
- Protocol schema regenerated and matches: pass.
- `git diff --check`: pass.
- `python3 tests/release_artifacts_test.py`: 6 tests pass.
- Built-in manual `permissions` topic and tracker JavaScript syntax: pass.

### Docs updated

`docs/feature/agent-version-policy.md` (now implemented),
`docs/feature/agent.md`, `docs/code/agent.md`, `docs/code/development.md`,
`docs/feature/deployment-config.md`, `docs/feature/deployment-operations.md`,
README, the website docs for the agent, security, server access, server layout,
CLI, quickstart, the built-in manual, the visual spec, the progress tracker,
and `.agents/task/agent-completion.md`.

### Remaining

- **Release version.** This changes the install, privilege, protocol, and
  execution contracts, so it needs a major release. The owner must choose and
  publish the version. Do not publish or push without that instruction.
- **Real major upgrade.** A CLI-major-greater connection was not run because
  the tree stays at `0.7.1`; the decision and cleanup are unit-tested and the
  cleanup path was exercised with an invalid older directory.
- **Not tested.** Alpine/busybox behavior, 32-bit ARM, disk full, and a real
  GitHub release download were not run. The no-`flock` directory lock was run
  in Podman, including two simultaneous installs.
- **Plain SSH code.** The old internal paths remain, but a connection no longer
  falls back to them after agent preparation or handshake failure.
- **Command-only sudo.** The agent installer and launch need root or full
  password-free sudo, so guided setup's `commands` mode cannot prepare or start
  the agent. The docs now say this. Whether to remove or replace that mode is an
  owner decision; nothing was widened silently.

## Agent version rules saved — 2026-11-08

The owner agreed the replacement access and compatibility policy in
`docs/feature/agent-version-policy.md`; persistent memory links to it.
Same-major minor/patch versions are compatible. A newer agent major refuses
the connection and reports the required agent version change. A newer CLI major
clears older agent installations and installs the required newer major.
Breaking protocol or execution/file-handling contract changes require a major
release. Agents live at root-owned `/opt/shipdeck/agent/v<version>/agent`, use
full executable paths, and come only from GitHub. No runtime change or cleanup
was performed. Current code still checks exact versions.

## Setup requirement removed from docs — 2026-11-08

Removed the required server-preparation step from the README, website entry
pages, quickstart, and server-access guide. Removed instructions to run or
rerun the server setup script from deployment, upgrade, troubleshooting,
planning, and terminal manual docs. Removed the per-server requirement wording
from the visual spec and progress page. Command references and historical
verification records remain. No replacement setup flow or runtime changes.

All nine terminal manual tests and `git diff --check` passed. Website build
failed because the MDX tooling tried
to create `/mnt/Projects`, which is not writable. No build configuration was
changed. Existing unrelated work was preserved.

## Agent install recipe — 2026-11-08

Added `just install-agent`, which runs `cargo install --path crates/agent
--locked` and installs the optimized binary into Cargo's bin folder. Added
local installation and SSH connection instructions to `docs/code/development.md`.
The development connection uses `SHIPDECK_AGENT_BINARY`; the engine uploads,
verifies, and starts the matching Linux agent over SSH.

Verified recipe parsing and command expansion with `just --list` and
`just --dry-run install-agent`; `git diff --check` passed. No local installation
or server connection was run. Existing unrelated changes were preserved.
Kanzo has no project linked to this repository, so no board task was updated.

## Commit checkpoint — 2026-11-08

The owner asked to commit the latest changes in focused steps. The changes
are grouped as protocol and schema, native agent operations, engine integration,
CLI/TUI controls, development recipes, and documentation with review records.
This checkpoint supersedes the older notes saying the tree is uncommitted.
No version change, push, or release publication is part of this task.

Fresh checks passed before committing: formatting, workspace compilation with
all targets, all 374 tests, strict pedantic/performance Clippy, and
`git diff --check`. Cargo was unavailable inside the sandbox; the Rust checks
ran outside it. SSH scenarios, schema generation, and release checks were not
rerun in this session; their earlier evidence and limits remain below.

## Follow-up review fixed — 2026-11-08

`docs/review/agent-completion-review.md` was extended with findings 7–10 after
the first six were fixed. All ten are now fixed and the review is `(done)`.

- **7, upload and service control.** `UPLOAD_PREPARE` and `SERVICE_CONTROL` now
  need the project lock. The upload carries the lock token; the receiver refuses
  a ticket when the lock is free or belongs to another token, and re-checks
  during the stream. `service/control` holds the lock for the whole change.
- **8, lost legacy lock channel.** The plain SSH lock shell is monitored. If its
  channel ends, `is_alive` fails and release reports shutdown or timeout errors.
- **9, commands outliving the agent lock.** Every `shipdeck.yaml` server command
  writes a `.shipdeck.command` lease with its shell PID and a token. A new
  deploy, rollback, cleanup or service control refuses while that command is
  alive; a stale lease is removed. This keeps a disconnected agent from letting
  a second operation overlap a command that is still running.
- **10, failed backup publish.** A non-duplicate publish error keeps the packed
  `.part` file and reports its path.

Verified after these fixes:

- Two new Podman failure tests. Killing the plain SSH lock shell during a
  deploy made it fail with "the connection holding the project lock was lost"
  and left the live release unchanged. Killing the agent during a long setup
  command made the next deploy say "a server command is still running for this
  project"; after the command ended, a later deploy succeeded.
- Unit tests cover an upload and a service control without the lock, a ticket
  after its lock is released, a kept backup part on a failed publish, and the
  mutation guard.
- The existing monorepo SSH suite passed after all lock and lease changes:
  deploy, unchanged, manager/app changes, shared data, retention, compressed
  backup rollback, failed restore, setup failure, partial activation recovery,
  custom PM2 backups, done warnings, and RPC controls/logs.
- Final sequence: formatting, workspace check, all tests (38 CLI, 43 agent,
  two agent process, 233 engine, 24 protocol, 34 TUI), strict Clippy, schema,
  `git diff --check`, and `dist generate --check`.

Remaining: publish a release with the four `shipdeck-agent-*` archives after the
owner picks the version; disk-full, 32-bit ARM and an actual GitHub release were
not tested. Upload speed still has no measured gain over SFTP or direct SSH.

## Agent completion continued — 2026-11-08

This section supersedes the stopping point below. The agent is now built and
verified for the tested single-app paths. The working tree is still
uncommitted; no commits were made. All older uncommitted work was preserved.

### What was finished

- **Compilation.** `apps/cli/src/console/run.rs` now uses the typed
  `ServerInfoGet`, `ReleasesList`, `WorkGet`, `EventsRead` and `ServerSetup`
  requests. No runtime dependency was added; `serde_json` remains a
  dev-dependency for tests. The protocol schema was regenerated.
- **Native agent operations.** Service status/control, release listing,
  release contents and file reads, release drops, live logs with stop,
  shared links, native backup create/restore, approved cleanup, failed-release
  discard, recent-event reads, and unconfirmed-activation detection.
- **Event recording and replay.** Build, pack, upload, unpack, setup, backup,
  switch, restart, health, final outcome and rollback events are numbered in
  `~/.shipdeck/state.db`. `shipdeck events --after N` replays a page; `work`
  shows the newest page.
- **Interrupted work.** `shipdeck work` shows the live release, service state,
  unfinished paths, recent events and an activation that never reached a
  health pass. `--cleanup` (or `x` in the TUI) is the approval; the server
  re-checks the list and refuses live releases, other releases, backups, and
  escaping paths.
- **Server facts.** `shipdeck server-info` and the TUI Server section show
  Linux, kernel, CPU, memory, disk, uptime, tools and agent version.
- **Guided setup.** CLI `shipdeck setup` and TUI `:setup` install missing
  tools, create the project folder, and install a `visudo`-checked sudoers
  rule. `--access full` grants password-free sudo for everything;
  `--access commands` grants only what shipdeck runs. The password travels on
  stdin and is never saved. Debian/Ubuntu and Fedora/RHEL are supported.
- **Review fixes.** All six findings in
  `docs/review/agent-completion-review.md` are fixed and the review is
  `(done)`: unique exclusive backup files, live-release checks before removal,
  real-directory checks, a shared project lock on the plain SSH path via
  `flock`, event database errors surfaced, and an 8 KiB bounded log reader.
  The agent also refuses project mutations unless the same connection holds
  the project lock, and `work --cleanup` takes that lock first.

### Verified after the final changes

- `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`,
  `cargo test --workspace` (all pass), strict pedantic/performance Clippy,
  protocol schema regeneration, `dist plan`, `dist generate --check`, and
  `git diff --check`. Logs: `.tmp/final-check.log`, `.tmp/final-test.log`,
  `.tmp/final-clippy.log`, and `.tmp/final-schema.log`.
- `shipdeck releases` on a fresh project with the agent prints
  `no releases on the server yet` and exits 0, as the plain SSH path did.
- Podman SSH (`127.0.0.1:44681`, key `.tmp/ssh/id_ed25519`): first and repeat
  single-app deploys; open and compressed rollback; shared-folder persistence;
  unchanged exit 6; native release list/contents/file reads; service
  stop/start; agent-followed logs with stop; guided setup for a non-root user
  in `commands` and `full` modes; a deploy without setup failing before the
  live release changed; interrupted upload with a bounded failure and kept
  partial; approved cleanup leaving the live release alone; a dropped
  activation connection exiting 5, then inspected with `work`; and mixed
  agent/legacy concurrency refusal in both directions.
- Plain SSH fallback with `SHIPDECK_AGENT_BINARY` pointing at a missing file:
  the deploy completed and held the `flock` project lock.
- The existing monorepo SSH suite passed after the lock, mutation-guard and
  path changes:
  `python3 tests/monorepo_ssh.py --container shipdeck-agent-work --port 44681
  --key .tmp/ssh/id_ed25519`, with `SHIPDECK_AGENT_BINARY` set. It covered
  deploy, unchanged, manager/app changes, shared data, retention, backup
  rollback, failed restore, setup failure, partial activation recovery,
  custom PM2 backups, done warnings, and RPC controls/logs. The printed test
  folder lived in the command's temporary `/tmp`, so its log was not kept.
- Release agent (`target/release/shipdeck-agent`, 5,738,456 bytes, 5.47 MiB on
  this machine): `--version`, `agent/hello`, `agent/ping`, `agent/shutdown` on
  the owner's VPS in an isolated `/tmp` folder. The folder was removed. No
  existing deployment, privilege, or Shipdeck state was changed.

Test logs and scripts are under `.tmp/e2e/` (ignored), including
`interrupted.log`, `drop.log`, `final-agent-deploy.log`,
`final-legacy-deploy.log`, `agent-first3.log`, `legacy-second3.log`,
`legacy-hold.log`, and `rpc.py`. The container `shipdeck-agent-work` still
runs at `127.0.0.1:44681` with root and `shipdeploy` (password
`ShipDeckTest!42`) for tests. `/opt/shipdeck-e2e`, `/opt/shipdeck-interrupt`,
`/opt/shipdeck-drop` and `/opt/shipdeck-user-e2e` are disposable test state.
The interrupted partial left in `/opt/shipdeck-interrupt` was kept on purpose.

### Remaining

- **Release.** Version stays 0.7.1 in this tree. A published release must carry
  the four `shipdeck-agent-*` archives; `dist plan` lists them. Decide the
  version and publish before claiming a public release.
- **Plain SSH policy.** p3d2 is still the owner's decision. The legacy path
  now takes the project lock but still uses shell commands.
- **Not tested.** Disk-full behavior, an actual GitHub release publication,
  and 32-bit ARM. Upload speed has no proven gain over SFTP/direct SSH.
- **Privileges.** Guided setup changes privileges only through the explicit
  `--access` choice. It was not run on the owner's VPS.

## Code review checkpoint — 2026-11-08

The tree changed after the agent-completion handoff below. It now compiles.
`cargo check --workspace --all-targets`, `cargo test --workspace`, formatting,
strict Clippy, and `git diff --check` passed in this review. The full Podman SSH
suite was not rerun on this tree.

Read `docs/review/agent-completion-review.md` before further agent work. It has
six open findings: backup partial overwrite, deletion of a live release,
incomplete project locking, cleanup through linked project directories, hidden
event-read errors, and unbounded log-line reads. Fix and test these before
calling the agent production ready. The earlier compile-failure and 802-line
notes below describe an older tree; `deploy/mod.rs` is now 775 lines.

## Agent completion handoff — 2026-11-08

The owner asked to hand this work to another agent. Stop implementation here.
The working tree is uncommitted. Read `.agents/task/agent-completion.md` first.
This section supersedes older notes below, including the old claims that
SQLite approval and the sudo choice are still open.

### First: restore compilation

The latest change started `server/info` and the CLI `server-info` command.
`cargo check --workspace --all-targets` currently fails: `apps/cli/src/console/run.rs`
uses `serde_json` in normal code, but `apps/cli/Cargo.toml` lists it only under
`[dev-dependencies]`. Fix this without silently adding an unapproved project
dependency: either use typed requests and answers without that new runtime
dependency, or obtain the owner's approval before promoting the existing crate.
The new `ServerFacts`/`ServerInfoGet` schema entries have not yet been written
to `docs/schema/protocol.schema.json`. Regenerate the schema after the CLI fix.
The facts code in `crates/agent/src/facts.rs` and the engine RPC were not yet
tested end to end. TUI server facts are not implemented. Review the `df`
parsing, missing PATH handling, and filesystem errors on supported Linux hosts.

`crates/engine/src/deploy/mod.rs` is 802 lines after these changes, above the
project's 800-line hard cap. Split by responsibility before final verification.
Run `cargo fmt --all -- --check`, the workspace tests, strict Clippy, schema
checks, and the Podman scenarios again after fixing the current work.

### Owner decisions and boundaries

- Continue until the agent is genuinely usable in production. Do not stop at a
  partial milestone. The owner explicitly asked another agent to continue.
- No `systemd-run`, detached deployment worker, scheduling, or public agent
  port. The agent runs over SSH as the connected user and exits on disconnect.
- Prepare and verify before activation. A dropped connection can leave files;
  do not clean interrupted work without the owner's approval. If activation
  was interrupted, inspect the actual live release and services.
- Preserve single-app and monorepo deployment, existing reuse, rollback, and
  the plain SSH path while the agent is developed. Whether plain SSH should
  remain supported in the long term is still undecided.
- The owner selected full sudo setup or a password-bypass option for guided
  setup. This is not implemented yet. Do not silently change a live server's
  privileges; the user explicitly allowed VPS tests without breaking anything.
- The owner approved `rusqlite` 0.41.1 with defaults disabled and bundled
  SQLite, plus the existing workspace `tar` and `zstd` crates for the agent.
  Ask before adding any other dependency. A small isolated tar/zstd size test
  added about 204 KiB, but it is not a measurement of the finished agent.
- Published builds download the matching agent. Development mode uses the
  explicitly selected `SHIPDECK_AGENT_BINARY`; it is not a download fallback.

### Work in the tree

- Explicit development upload checks ELF CPU, uploaded SHA-256, exact agent
  version, and the agent protocol. Agent protocol is now 2. Version remains
  0.7.1; assess release versioning and cached older binaries before publishing.
- A dedicated SSH channel sends release bytes to the same agent in receiver
  mode. It uses a private one-use reservation, bounded buffers, declared size,
  checksum, and a confirmation. Partial files remain after failure. First
  attempts refuse existing unfinished artifacts; the same operation may retry
  a full upload after a reuse mismatch.
- Agent-native hashing, project directory preparation, atomic current-link
  switching, and tar/zstd extraction are connected to single-app and monorepo
  deployment. Extraction checks paths, symlinks, file modes, byte and entry
  limits, checksums, and reuse. It keeps failed staging for inspection.
  Service management, shared links, backups, and other operations still use
  existing SSH commands; native file and service work is incomplete.
- Project locks are held by the connected agent using OS file locks. Tokens
  prevent a cancelled request from unlocking another operation. Both deploy
  shapes and rollbacks acquire locks and refresh live facts. The engine checks
  the lock during the pipeline and reports uncertain activation on a lost
  connection. Locks only cover cooperating agent sessions; the legacy SSH path
  does not participate. A command already running over SSH may outlive a lost
  control channel. Test and resolve this before claiming production safety.
- `~/.shipdeck/state.db` stores numbered events with SQLite. Upload start and
  verification, unpack start/result, and switch start/completion are stored.
  Bounded event replay exists at the agent/engine layer. Full deployment event
  recording, client replay, and uncertain-state recovery are still missing.
- Agent inspection reports the live symlink and unfinished files. Preparation
  warns about them. There is no complete reconnect view or approved cleanup
  flow yet. Check all legacy rollback and checksum-mismatch cleanup paths
  against the owner's rule before finishing.
- The CLI `releases` command previously hung when a new host key was asked
  during its request. It now handles questions while waiting. The subsequent
  incomplete generic request refactor is part of the compile failure above.

### Verified and not verified

- Latest `cargo test -p shipdeck-agent --lib`: 17 passed, including archive,
  SQLite, locks, and facts parser tests. Latest workspace `cargo check` failed
  as described above. Do not report current workspace tests or Clippy as
  passing; they passed before later changes, not on this exact tree.
- The full monorepo Podman deployment/rollback suite passed with native
  extraction: `/tmp/shipdeck-monorepo-e2e-e8uxmk31/commands.log`. It started
  before the latest retry, event, and server-info changes; rerun it.
- `/tmp/shipdeck-agent-work/concurrent_deploy.py` passed: one deployment
  succeeded and a concurrent one was refused. `ssh_lock_test.py` checked two
  SSH agents, wrong tokens, and release on disconnect. `interruption.py`
  checked a truncated upload, replay refusal, and an unchanged live link.
  `cancel_agent.py` checked a blocked SQLite request ends on disconnect.
- The disposable container `shipdeck-agent-work` is running on
  `127.0.0.1:44681`; key: `/tmp/shipdeck-agent-work/id_ed25519`. Check before
  reuse. Podman requires an escalated command in this environment. The owner
  authorized Podman testing. It contains test projects and processes.
- The owner allowed `ssh mia_namecheap` only without breaking existing
  deployments. It is root on Ubuntu 26.04.1 LTS, Linux x86-64. A private temp
  folder held a test binary and upload measurements; all those completed test
  files and the folder were removed. No existing deployment or Shipdeck state
  was changed. Do not perform deployment or privileged setup on that VPS.
- VPS transfer trials used one connection and 8 MiB, with a server checksum:
  SFTP 36.7–49.2 s, direct SSH 35.7–59.4 s, command RTT 699–756 ms. The agent
  receiver was not measured on that VPS. The separate local Podman benchmark
  did not show an agent speed gain. Do not claim faster uploads without better
  evidence. Logs and sources: `/tmp/shipdeck-agent-work/`.

### Remaining work

1. Fix the current compile failure and schema; then finish and verify server
   facts in the CLI and TUI. Update the docs and tracker.
2. Finish agent-native file operations, shared links, backups, service controls,
   releases, and logs. Audit archive extraction and cancellation, including
   concurrent sessions, filesystem races, symlinks, disk full, and retries.
3. Implement complete numbered event recording and reconnect replay; inspect
   uncertain activation and provide cleanup only after explicit user approval.
4. Build guided setup and the approved full sudo/password-bypass option. Test
   fresh and existing Ubuntu/Debian servers in Podman; do not modify the VPS.
5. Verify both deployment shapes, dropped SSH during upload, preparation and
   activation, user-approved cleanup, permission failures, and release assets.
   Measure actual binary size and any upload gains. Run the final required
   checks and update the plan, tracker, feature docs, visual spec and handoff.

Other uncommitted work predates agent implementation, including README and
justfile install recipes and the Cargo.lock crypto-bigint update. Preserve it;
do not reset or discard unrelated changes. No commits were made in this run.

Current checkpoint: 2026-11-07.

## Agent SQLite dependency review — 2026-11-07

Also compared libsql 0.10.0 with defaults disabled and core only: 69 Linux
normal/build package-version pairs, 24 absent from the current lockfile.
Minimal bundled rusqlite resolves 11 total and five new pairs. libsql supplies
an async-shaped local/remote API but also links its own native libSQL engine.
Comparison added to the review; no project dependency added. Choice remains open.

The owner paused SQLite dependency approval to check size and dependency cost.
Review: docs/review/agent-sqlite-dependency-review.md. Isolated offline trees
for rusqlite 0.41.1, with defaults disabled, add five workspace package names
with or without bundled: rusqlite, libsqlite3-sys, both fallible iterators,
and vcpkg. Bundled compiler helpers already exist elsewhere in the workspace.
Only smallvec is already in the agent tree. No binary-size measurement or
project dependency addition was made. Evidence is under
/tmp/shipdeck-rusqlite-review-qqkxzryw/. Approval remains pending.
The owner selected an unrestricted sudo/password-bypass setup option for
future server operations. This supersedes the earlier narrow-permissions
recommendation; implementation and testing are still pending.
Agent work began before the dependency question: local development binary
upload changes are uncommitted in engine/agent/development.rs and its callers.
The earlier test run was interrupted; do not report those checks as passed.

## Agent working plan — 2026-11-07

Start future agent work at .agents/task/agent-completion.md. Its A1–A11 table
tracks implementation and verification. No implementation started this session.
The owner's newer decisions supersede older durable-job notes below: no
systemd-run or detached deploy worker; prepare first, activate last; leave
interrupted files until cleanup is approved. Reconnect must inspect an
activation whose result is uncertain. Development mode explicitly uploads a
matching local agent; never use it as a failed-download fallback.
The proposed data channel runs the same binary in a short-lived receiver mode,
separate from control JSON. Receiver coordination and upload benchmarks are
still design work. Updated agent guide, tracker scope, and visual spec.

## Agent completion scope review — 2026-11-07

The owner reported an agent connection failure on a VPS and asked what would
be built to complete the agent. Checked the current agent loop, engine startup,
installer, release configuration, Phase 3 tracker, and agreed visual spec.
The binary exists but serves only hello, ping, and shutdown. Remaining work
includes native file operations, durable deploy jobs, numbered SQLite events,
server setup, server information, engine/client integration, and release checks.
The existing handoff records missing agent assets in v0.7.1. Current code still
requests that tag. Live release verification failed: gh could not connect to
api.github.com, and the web tool could not access the release API. The exact
VPS failure remains unconfirmed without its error output. No runtime code changed.
Root permissions remain an owner decision. Desktop Servers UI belongs to the
desktop work; agent/server RPC support belongs to agent completion. Nginx,
databases, Docker apps, mail, and keepup remain separate feature phases.

## Yanked cryptography dependency — 2026-11-07

The install warning came from crypto-bigint 0.7.5 pinned in Cargo.lock.
Its changelog cites security fixes and later API changes as yank reasons.
RustSec RUSTSEC-2026-0387 describes timing variability fixed in >=0.7.6:
https://rustsec.org/advisories/RUSTSEC-2026-0387.
Updated only crypto-bigint to 0.7.10 and its required ctutils to 0.4.8 with
`cargo update -p crypto-bigint --precise 0.7.10`. Kept --locked in recipes.

Formatting, workspace/all-target compilation, strict Clippy, and diff checks
passed. CLI tests (37), engine tests (228), and agent library tests (6) passed.
Full workspace testing remains limited by cached absolute paths from the old
/mnt/Projects checkout: two agent process tests cannot spawn their binary;
the protocol schema test reads its old manifest path and gets no file.
No application source or manifest changed. No global reinstall was run.
Additional test output: /tmp/shipdeck-crypto-update-tests.log.

## Local install recipe — 2026-11-07

Added `just install`: `cargo install --path apps/cli --locked`. It builds the
local CLI in release mode and installs it into Cargo's configured bin folder,
usually ~/.cargo/bin. README development examples include the command.
Verified recipe parsing, dry-run output, justfile formatting, and diff checks.
The installation itself was not run; the request was to add the recipe.

## Justfile essentials — 2026-11-07

Added safe default help, debug/release run recipes, workspace builds with a
profile argument, build-release/release shortcuts, check, test, fmt, fmt-check,
strict lint, verify, API docs, clean, and a website build recipe. Existing schema,
protocol, version, and website commands remain. README development examples
describe the new commands. Arguments use quoted shell positional parameters.

Verified: default help, real `just run --help`, justfile formatting, diff checks,
nine isolated argument-forwarding checks, workspace formatting, compilation,
and strict pedantic/performance Clippy. `just test` reaches existing agent
process tests, but both fail to spawn their compiled-in binary path with
ENOENT. The CLI's 37 tests and agent library's six tests pass before this
failure. The full workspace suite therefore did not pass. No application code
or dependencies changed; no release build or website deployment was run.

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

## Flaky lock tests fixed — 2026-11-08

`cargo test --workspace` failed at random in
`transfer::tests::an_upload_ticket_stops_working_when_its_lock_is_released`,
and sometimes in
`locks::tests::project_lock_blocks_other_sessions_and_is_released_on_drop`.

Cause: `flock` belongs to the open file description. Tests run in parallel, and
many agent tests call `owner::apply`/`apply_tree`, which spawn `id` or `getent`.
Between `fork` and `exec` the child copies every descriptor, so a lock released
with `drop` still looks held for that window. Traces showed the failed probe
inside `SPAWN_BEGIN`/`SPAWN_END` for `id`.

Fix: test-only `crates/agent/src/test_support.rs::wait_until_unlocked` waits for
the lock file to become lockable before the release assertions. No production
code changed. Evidence: `.tmp/trace/fail.log`.

Checks: 100 parallel agent-lib runs and 10 serial runs pass. Workspace tests
(38 CLI, 48 agent, 2 agent process, 249 engine, 24 protocol, 34 TUI),
formatting, and strict workspace Clippy pass.

The production receiver has the same narrow window: an agent that spawns a
command while it releases a project lock can make a stale upload ticket look
valid for the fork-to-exec interval. Not changed; it needs an owner decision.
