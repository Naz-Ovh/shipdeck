# Agent completion plan

Updated: 2026-11-08. Status: built and verified; the privileged install policy
is implemented, and a major release with the agent archives remains.

This is the working plan for completing the server agent. Read it before
agent work. Keep this checklist current, along with the project tracker and
`.agents/handoff.md`. Existing tracker IDs stay permanent.

## Privileged install and version policy

Implemented on 2026-11-08, from
[the agreed policy](../../docs/feature/agent-version-policy.md):

- The first connection prepares the agent. The account must be root or have
  password-free sudo.
- The agent lives at `/opt/shipdeck/agent/v<version>/agent`; root owns it and
  every parent directory. It starts through sudo with the full path.
- Same-major minor and patch versions are compatible. A newer agent major
  refuses the connection. A newer engine major clears older versions and
  installs its release.
- Installs are locked, checksummed, staged privately, and published in one
  rename. Older version directories are removed after a successful publish.
- App and PM2 commands carry an explicit service user. Deployment files are
  given back to the SSH user.
- A failed install or handshake ends the connection. No operation runs through
  an incompatible agent.

The current tree stays at `0.7.1`. This changes the install, privilege, and
execution contracts, so it needs a major release. The owner must choose and
publish that version.

## Agreed scope

- The agent runs as root through sudo on a versioned `/opt` path and opens no
  public port. App and PM2 commands use the configured service user.
- One SSH connection carries requests, events, and file data in separate channels.
- Published builds download the release for the connecting engine over HTTPS.
  An installed agent with the same major version is reused.
- Development mode explicitly uploads a local agent over SSH. It is not a
  fallback for failed release downloads. Check CPU, Linux target, protocol,
  and engine version before using it.
- Deploys run while connected. No `systemd-run`, detached deploy workers,
  scheduling, or automatic continuation after a disconnect.
- Prepare and verify the release before final activation. Activation includes
  switching the live release, service changes, and health checks.
- A disconnect during preparation leaves the live release unchanged. Keep
  unfinished files and offer cleanup later, only with user approval.
- A disconnect during activation can leave an uncertain result. Inspect the
  actual live release and services on reconnect; never report success by guess.
- Keep systemd and PM2 for running deployed applications.
- Keep existing single-app and monorepo behavior, backups, and rollback.
- Nginx management, databases, ready-made apps, mail, and keepup are separate
  phases. Desktop page implementation stays in the desktop work.

## Verified starting point

The agent binary and library exist in `crates/agent`. Engine startup and
privileged installation are in `crates/engine/src/agent`. A startup or
handshake failure now ends the connection instead of leaving a plain SSH path
available.

`Session::upload` in `crates/engine/src/ssh/mod.rs` uses SFTP first, with a
shell-stream fallback. It feeds 256 KiB chunks and configures concurrent SFTP
writes. No benchmark here proves which part limits real VPS upload speed.
Single-app and monorepo deployment already pack compressed bundles and reuse
unchanged files. Preserve that work.

The handoff records missing agent archives in release v0.7.1. Live GitHub
verification failed during the earlier review. The owner's exact VPS failure
still needs its full error message or a reproduction.

## Separate SSH upload channel

Proposed layout, pending the transfer benchmark and protocol review:

```text
Engine                    One authenticated SSH connection          VPS
  |                                                                |
  +-- control channel: requests / replies / events <-----------> agent
  |                                                                |
  +-- upload channel: compressed binary bytes -------------> receiver
                                                               |
                                                        staging folder
```

A channel is a separate stream inside the existing SSH connection. It does
not need another login, network port, or relay. Both channels share the same
network connection and bandwidth. Separating them avoids putting file bytes
in JSON; it does not guarantee logs stay fast when the link is saturated.

The control agent's stdin belongs to its own channel. A second SSH channel
cannot write into that stdin. Proposed solution: start the same agent binary
in a narrow upload-receiver mode on the data channel. This receiver exists
only for the transfer. It is not a background deploy worker.

1. The engine sends a typed preparation request with the release manifest,
   file sizes, checksums, and reuse candidates.
2. The agent validates the project root and reserves a unique staging area.
   It checks reusable files and returns a transfer identifier and limits.
3. The engine opens an SSH exec channel for the fixed receiver mode. Bind
   the receiver to that transfer and SSH user. Refuse invented, expired,
   already claimed, or cross-project identifiers.
4. The engine streams the compressed bundle. Keep bytes out of JSON. Use
   bounded buffers and respect SSH backpressure: slow receivers must slow
   the sender instead of growing memory without limit.
5. The receiver validates length, checksum, and stream completion. Verify
   extracted file paths, sizes, hashes, and links before marking staging ready.
6. The engine requests final activation only after preparation succeeds.

The framing, transfer identifier format, receiver coordination, and channel
limits are design work in A3. Do not treat this sketch as an implemented API.
Start with the existing bundle format. Per-file parallel streams, resumable
uploads, and block-level differences are not approved additions.

## Work and progress

States: pending, doing, built, verified. Built means code exists; verification
must name actual checks. Do not stop at built when required work remains.

| Step | Work | Tracker | State | Completion evidence |
| --- | --- | --- | --- | --- |
| A1 | Release installation and explicit local development upload. | p3a1 | verified | Real SSH install with a development binary; exact version/target/checksum checks; a dead upload receiver now fails in seconds instead of hanging. |
| A2 | Typed native file operations and staging boundaries. | p3a2, p3a3 | verified | Native hashing, extraction, shared links, backups, restore, release listing/reading, and approved cleanup; unit tests and Podman deploys passed. |
| A3 | Upload benchmark, protocol, and dedicated SSH receiver. | p3a3 | doing | Separate channel and verification passed earlier checks. Follow-up review found upload reservation is allowed without the project lock. |
| A4 | Agent deployment preparation for single apps and monorepos. | p3a3 | verified | Compression, reuse, shared links, native backups, and saved settings work through the agent; repeated single-app deploys and the existing monorepo SSH suite passed in Podman. |
| A5 | Final activation, health checks, and rollback. | p3a4 | doing | Rollback and dropped-connection checks passed. The lock lifetime during an in-flight SSH command still needs a failure test and fix. |
| A6 | Interrupted-work inspection and approved cleanup. | p3a4 | verified | `work` lists current, service, unfinished paths, recent events, and unconfirmed activation; `--cleanup`/`x` is the approval; the agent refuses unreported, live, or escaping paths. |
| A7 | Saved deployment records and numbered events in state.db. | p3b1 | verified | Build, pack, upload, unpack, setup, backup, switch, restart, health, outcome, and rollback events are numbered and replayable with `shipdeck events --after`; uncertainty is read from them. |
| A8 | Service controls, releases, and live logs. | p3a2, p3a3 | doing | Service controls work, but can change a service without the project lock. Other typed operations passed earlier SSH checks. |
| A9 | Guided server setup and privileges. | p3c1, p3d1 | verified | Debian/Ubuntu and Fedora/RHEL path; password on stdin; missing packages, project folder, and visudo-checked rule. Full and command-only access tested on a non-root user in Podman. |
| A10 | Server facts through protocol and CLI/TUI. | p3c2 | verified | `shipdeck server-info` and the TUI Server section show real Linux, kernel, CPU, memory, disk, uptime, tools, and agent version. |
| A11 | Full integration, release checks, and documentation. | All above | doing | Local checks and earlier SSH scenarios passed. Follow-up lock findings need repairs and new failure tests before release. |

The main tracker rows for p3a1 and p3a2 are built and now cover the
development-upload and native deployment APIs described here.

## Upload measurement

Compare current SFTP, existing direct SSH streaming, and the agent receiver
against the same server and identical payload. Record payload bytes, checksum,
round-trip latency, elapsed transfer time, throughput, and peak memory.
Measure packing, remote verification, and unpacking separately.
Test both a large bundle and a release with many small source files. Include
first upload and reuse-heavy updates. Keep build profile and compression
settings equal. Repeat trials and record variation. Do not add an HTTP server,
new transport, or multiple upload connections without evidence and approval.

## Decisions still open

- The major release version. The owner must choose it; do not select or publish
  it without that instruction.
- Confirm the supported setup distributions and required package list.
- Development builds use the explicit `SHIPDECK_AGENT_BINARY` path. Never
  automatically upload an unrelated host build to the VPS.
- The old plain SSH code remains internally, but a connection no longer uses it
  to bypass agent preparation.

The owner chose `rusqlite` with bundled SQLite and default features disabled.
The owner also chose full sudo setup or a password-bypass option.
The owner approved reusing `tar` and `zstd` in the agent for native extraction.

## Verification and continuation

For each completed step, add evidence below: date, changed paths, commands,
results, and remaining limits. Update its row here and the related main tracker
row when that step's full scope is built. Keep the tracker's Up next box and
UPDATED current. Update the visual spec when an agreed design changes.

Run `cargo fmt --all -- --check`, relevant tests, and
`cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf`.
Run workspace tests for final integration. Test real SSH transfers and deploys,
including dropped connections during upload, preparation, and activation.
Check concurrent attempts against the same project and disk-full failures.
Never claim an unrun check passed or an unmeasured speed gain.

Next: the owner chooses a major version and publishes a release with the four
`shipdeck-agent-*` archives. All ten findings in
`docs/review/agent-completion-review.md` are fixed and the review is `(done)`.

## Evidence log

- 2026-11-08: Implemented the privileged install and version policy. Podman
  tests covered first install, reconnection, same-major `0.6.9`, newer-major
  refusal, older-directory cleanup, simultaneous installs, interrupted
  download, bad checksum, wrong version, failed handshake, root ownership,
  replacement attempts by ordinary users, development upload, missing sudo,
  PM2 and systemd deploys, a setup command running as `shipdeploy`, and
  non-interactive CLI use. Unit tests cover the version choices and the
  CLI-major cleanup decision. Formatting, workspace tests, and strict Clippy
  pass. The release version is not selected.
- 2026-11-07: Added explicit local agent upload and a typed operation for
  hashing files in the live release. Single-app and monorepo reuse checks now
  call the agent when connected. Agent unit tests and the development-upload
  tests pass. Full SSH and failure-path checks are still in progress.
- 2026-11-07: Added a separate receiver channel for release bundles. Unit tests
  cover short, oversized, altered, and repeated uploads. Real SSH testing and
  performance measurements remain open.
- 2026-11-07: Added bundled SQLite, numbered event append and paged replay.
  Upload start and verification now record events. Other deployment events,
  reconnect integration, and interrupted-state recovery are still open.
- 2026-11-07: Podman SSH tests deployed a new and an updated monorepo release
  through the receiver. The database recorded upload start and verification.
  A truncated stream kept its partial file, refused ticket reuse, and left
  `current` unchanged. The wider monorepo SSH suite also passed, although it
  started before the newest receiver build. Logs and reproduction files are
  under `/tmp/shipdeck-agent-work/` and `/tmp/shipdeck-agent-channel-xbxc1bf4/`.
- 2026-11-07: The agent can inspect the live link and unfinished release files.
  Deployment preparation warns about them. A full reconnect view and approved
  cleanup are not implemented yet.
- 2026-11-07: A local Podman measurement sent 8 MiB of identical random bytes
  three times per method. After reducing request round trips, SFTP took
  0.170–0.179 s, direct SSH took 0.122–0.253 s, and the agent took
  0.304–0.355 s. Each timing included a separate server checksum check; the
  agent also checks bytes while receiving, syncs the file, and saves events.
  These short local tests do not establish real VPS speed. Logs and the
  measurement source are in `/tmp/shipdeck-agent-work/`. Latency, memory per
  method, large releases, packing, unpacking, and reuse still need measurement.
- 2026-11-07: Native project directory preparation and atomic live-link
  switching are in progress. Switch start and completion are saved as events.
  The existing service and health steps remain in the engine.
- 2026-11-07: On the owner's VPS, a release agent uploaded to an isolated
  `/tmp` folder matched its local checksum and answered hello, ping, and
  shutdown over SSH. No existing deployment or Shipdeck state was changed.
- 2026-11-07: A Podman deployment used native directory preparation and the
  atomic live switch. Its upload and switch events appeared in sequence in
  SQLite. It reported the earlier interrupted upload without deleting it.
- 2026-11-07: The owner's VPS received identical 8 MiB files three times over
  SFTP and three times over direct SSH, using one connection. Each time includes
  a server checksum check. SFTP took 36.7–49.2 s; direct SSH took 35.7–59.4 s.
  The measured command round trips were 699–756 ms. The link varied during
  this test. The agent receiver itself was not benchmarked on this VPS.
- 2026-11-08: Two concurrent Podman deployments targeted one project. One
  completed and the other was refused by the project lock. Independent SSH
  agent sessions also showed that disconnect releases the lock and a wrong
  unlock token cannot release it.
- 2026-11-08: Added native tar/zstd extraction with size and entry limits,
  path and link checks, reuse verification, and a full-upload retry. Unit tests
  cover changed reuse files, oversized archives, and safe and unsafe links.
  An updated monorepo release deployed successfully through native extraction.
  The complete deployment suite and more failure cases are still running.
- 2026-11-08: The full Podman monorepo deployment and rollback suite passed
  with native extraction. Its log is
  `/tmp/shipdeck-monorepo-e2e-e8uxmk31/commands.log`. Changes made after the
  suite started still need a fresh end-to-end run.
- 2026-11-08: The owner's VPS reports Ubuntu 26.04.1 LTS, with systemd and
  sudo present. This was checked read-only.
- 2026-11-08: A Podman SSH test blocked an event write in SQLite, then closed
  the agent input. The agent exited in 0.02 seconds, and the blocked event was
  not written later. Test: `/tmp/shipdeck-agent-work/cancel_agent.py`.
- 2026-11-08: Restored compilation without a new runtime dependency by using
  the typed `ServerInfoGet`, `ReleasesList`, `WorkGet`, `EventsRead`, and
  `ServerSetup` requests; regenerated the protocol schema.
- 2026-11-08: Added native agent service status/control, release listing and
  inspection, live logs with notifications, shared links, backup create and
  restore, release drops, approved cleanup, recent-event reads, and an
  activation-uncertainty check. The engine falls back to the old SSH commands
  when the agent is unavailable.
- 2026-11-08: Added deployment event recording, `shipdeck events --after`,
  `shipdeck work` and `work --cleanup`, TUI Server/Work sections, `x` cleanup,
  `:setup`, and CLI/TUI guided setup with `--access full|commands`.
- 2026-11-08: Podman verification passed: first/repeat deploys, open and
  compressed rollback, shared data, unchanged exit 6, native release reads,
  service stop/start, live logs, guided setup for a non-root user in both
  access modes, permission failure before setup, interrupted upload with a
  bounded failure and kept partial, approved cleanup, and a dropped activation
  connection that exited 5 and was inspected with `work`.
- 2026-11-08: The plain SSH path passed with `SHIPDECK_AGENT_BINARY` pointing
  at a missing file. The release agent answered `--version`, hello, ping, and
  shutdown in an isolated `/tmp` folder on the owner's VPS; the folder was
  removed and no existing deployment or Shipdeck state was changed.
- 2026-11-08: Final checks passed: formatting, workspace compilation, all
  tests, strict pedantic/performance Clippy, protocol schema, `dist plan`,
  and `dist generate --check`. Release agent size on this machine: 5,738,456
  bytes (5.47 MiB), stripped with thin LTO.
- 2026-11-08: Fixed all six findings in the agent completion review: unique
  backup files with exclusive creation and link-based publish; live-release
  checks before removal; real-directory checks for project folders; `flock`
  on the plain SSH project lock and mixed-agent concurrency tests; event
  database errors surfaced by `work`; and an 8 KiB bounded log line reader.
  The review is `(done)`.
- 2026-11-08: Final sequence passed with strict exit status: formatting,
  workspace check, all tests, strict Clippy, schema regeneration,
  `git diff --check`, and `dist generate --check`. `shipdeck releases` on a
  fresh project prints `no releases on the server yet` and exits 0.
- 2026-11-08: The existing monorepo SSH suite passed after the lock and path
  changes, with the agent enabled. It covered deploy, unchanged, manager/app
  changes, shared data, retention, compressed backup rollback, failed restore,
  setup failure, partial activation recovery, custom PM2 backups, done
  warnings, and RPC controls/logs.
- 2026-11-08: Fixed follow-up findings 7–10: upload and service control now
  need the project lock; upload tickets are bound to the lock token and
  re-checked during the stream; the plain SSH lock shell is monitored and
  release errors are reported; server commands use a `.shipdeck.command` lease
  so a disconnected agent cannot let a second operation overlap them; a failed
  backup publish keeps the packed part. Podman failure tests killed the lock
  shell and the agent during a long command; the live release stayed unchanged,
  the second deploy was refused, and a later deploy succeeded. The monorepo
  suite passed after the changes.
- 2026-11-07: Plan created from the owner's decisions and current source.
  No agent implementation or upload benchmark has been performed in this step.
