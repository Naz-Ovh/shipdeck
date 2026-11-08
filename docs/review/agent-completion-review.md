# (done) Agent completion review

## Scope

Reviewed the uncommitted server-agent implementation, its engine and client
callers, the current agent plan, and the saved handoff. This review focuses on
file safety, deployment concurrency, interrupted work, and live inspection.
The tree has not been deployed to a production server in this review.

## Evidence

- `cargo check --workspace --all-targets`: passed.
- `cargo test --workspace`: passed, including 33 agent library tests and two
  agent process tests.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf`:
  passed.
- `git diff --check`: passed.
- Read the agent, engine, CLI, TUI, protocol, and setup paths named below.

## Findings

### 1. Interrupted backup data can be overwritten (high)

`crates/agent/src/backup.rs:43-49` gives every backup attempt the same hidden
`.part` path and opens it with `File::create`. A second attempt for the same
release truncates the unfinished backup. `File::create` also follows an existing
symbolic link, so a link at that path can truncate a different writable file.
The final `rename` at line 67 can replace a backup created after the earlier
existence check. This breaks the agreed rule to preserve interrupted work.
Create a unique private file exclusively, and publish it only if the final name
is still absent. Add tests for an existing partial, a symbolic link, and a
concurrent final backup.

### 2. Removal does not check the live release (high)

`crates/agent/src/inspect_release.rs:113-123` removes an open release without
checking `current`. `crates/agent/src/cleanup.rs:35-44` does the same for a
failed release. The engine's retention plan avoids the release that was current
when it listed the server, but a second client or the plain SSH path can switch
`current` before removal. Removing the new live folder then breaks the app.
Check the live link at the point of deletion and refuse its target. Coordinate
removal with switching under the project lock.

### 3. Project locking does not cover every writer (high)

`crates/engine/src/deploy/mod.rs` and `multiple.rs` acquire an agent lock only
when an agent is present. The plain SSH fallback still performs deployment and
rollback without that lock. Agent methods such as `switch/release`,
`release/drop`, and `work/cleanup` do not require a lock token either. Thus two
clients can write to the same project despite the new lock; a remote shell
command may also continue after the agent connection drops and releases it.
Make all mutation paths use one project-wide lock and prove the behavior with
mixed agent/plain SSH clients and a dropped control channel.

### 4. Cleanup can follow a linked project subdirectory (medium)

`crates/agent/src/cleanup.rs:20-29` joins an approved relative path to an
absolute root, then removes it. It does not verify that `releases/` or
`backups/` is a real directory. `work/get` also enumerates through either
directory if it is a link. If `releases/` points elsewhere, approved cleanup
of `releases/.x.partial` removes data at that other location. Reject linked
project subdirectories and keep the directory identity stable during deletion.

### 5. A database failure looks like an empty event history (medium)

`crates/engine/src/rpc/work.rs:30-35` uses `unwrap_or_default()` when the agent
cannot read recent events. The user then sees a normal `work/get` result with
no events, even if `state.db` is locked or corrupt. Those events are needed to
understand an interrupted activation. Return the error or show an explicit
warning in the report.

### 6. The log line limit does not limit memory (medium)

`crates/agent/src/logs.rs` reads a whole line with `BufReader::lines()` before
calling `truncate` at 8 KiB. One service log line can therefore allocate far
more than 8 KiB in the agent. Read bounded chunks or use a bounded line reader;
test a large line before claiming a memory bound.

## Resolution

The first follow-up marked all six findings fixed on 2026-11-08. Those code
changes remain in the working tree. The later review found that the lock fix
in finding 3 does not cover every mutation or lock-loss path; see below.

1. **Backup overwrite.** `crates/agent/src/backup.rs` now refuses any existing
   unfinished backup for the release, creates a unique private `.part` file
   with `create_new` and mode `0600`, and publishes it with `hard_link`, which
   cannot replace an existing name. A symbolic link at the final name is left
   untouched. Tests cover an existing partial, a symbolic link, and an existing
   final backup.
2. **Live-release removal.** `crates/agent/src/cleanup.rs` and
   `crates/agent/src/inspect_release.rs` call `paths::refuse_current` before
   removing anything. Tests cover dropping and discarding the live release.
3. **Project locking.** Both paths now take the same `.shipdeck.lock`:
   the agent uses its OS file lock, and the plain SSH path holds `flock` on a
   dedicated channel for the whole run. `flock` is required on the legacy path
   at preflight. Lock acquisition retries briefly so an agent that just exited
   cannot cause a false refusal, and `Agent::shutdown` waits for the stream to
   end. The agent refuses every project mutation (prepare, unpack, switch,
   link, backup, restore, cleanup, drop and discard) unless the same connection
   holds the project lock. `work/cleanup` takes the lock before it checks and
   removes. Podman tests proved two concurrent legacy deploys are refused, an
   agent deploy blocks a legacy one, and a legacy deploy blocks an agent one;
   unit tests prove an agent mutation is refused without the lock.
4. **Linked project folders.** `crates/agent/src/paths.rs` checks that
   `releases/`, `backups/`, `shared/` and each release are real directories
   directly under the project; inspection, cleanup, backups, release reads and
   removal use it. Tests cover a linked `releases/` and a linked backup.
5. **Event database errors.** `work/get` now returns the agent's error instead
   of an empty event list.
6. **Log memory.** `crates/agent/src/logs.rs` reads in fixed 8 KiB chunks and
   keeps at most 8 KiB per line, discarding the rest until the newline. Tests
   cover a line longer than the limit, a huge line without a newline, and the
   following line.

Verified after the fixes: `cargo fmt --all -- --check`, workspace tests,
strict pedantic/performance Clippy, protocol schema, `dist plan`,
`dist generate --check`, and the Podman SSH scenarios named in
`.agents/handoff.md`. Disk-full behavior and actual release publication were
not tested; the release workflow and its four agent archives were checked with
`dist`, not by publishing.

## Follow-up review — 2026-11-08

The earlier file-safety and event fixes remain in place. A new source review
and runtime check found the following open issues. Findings 7–9 reopen the
locking concern in finding 3.

### 7. An upload can be reserved without the project lock (high)

`crates/agent/src/dispatch.rs:146-154` lists mutating methods that need the
lock, but omits `UPLOAD_PREPARE`. A direct request to the current agent binary
returned an upload ticket without first locking the project. The separate
receiver can then publish a release archive on another SSH channel. The same
list omits `SERVICE_CONTROL`, and `crates/engine/src/rpc/service.rs:93-104`
does not lock before changing a service. Another client can therefore upload
or stop a service while a deployment holds the project lock. Guard these paths
and test concurrent control and upload attempts. A ticket must also stop being
usable after its owning operation releases the lock.

### 8. A lost lock channel is not detected by plain SSH deploys (high)

`crates/engine/src/agent/lock.rs:119-126` checks the SSH session for legacy
deploys, but does not check the dedicated `flock` channel. If that channel's
remote shell exits while SSH remains connected, the OS releases the lock and
the first deploy keeps working. A second deploy can then take the lock and
overlap it. At lines 148-154, explicit release also ignores both channel
shutdown and exit-wait errors. Monitor the lock channel throughout the run and
fail before more project changes if it ends; make release failure visible.

### 9. An agent disconnect can release a lock before SSH commands finish (high)

The agent owns the file lock for its control connection. On disconnect it
releases that lock, but deployment commands run on separate SSH channels.
`crates/engine/src/ssh/mod.rs:39-41` notes that closing a channel does not stop
a non-PTY process. A command already changing services or files may continue
after the agent lock disappears, allowing a second client to start. The Podman
disconnect check verified an uncertain result, not exclusion from a second
deployment while a command remains active. Keep the lock until those commands
are known to have ended, or make the command lifetime enforce that boundary.

### 10. A failed backup publish deletes the prepared backup (medium)

`crates/agent/src/backup.rs:73-84` removes its complete `.part` file whenever
`hard_link` fails, including permission and filesystem errors. This loses the
backup that was just packed, despite the rule to keep interrupted work for
inspection, and it discards a filesystem removal error. Keep the part on
non-duplicate failures; report any cleanup failure explicitly.

### Current verification and limits

`cargo test --workspace -q` passed: 38 CLI, 40 agent, two agent process, 233
engine, 24 protocol, and 34 TUI tests. Formatting, strict Clippy, and
`git diff --check` passed. A direct local agent request confirmed finding 7.
The full Podman SSH suite and release publishing were not rerun in this
follow-up review. Findings 8 and 9 follow from the channel and process
lifetime code; they still need failure-injection tests.

## Resolution of the follow-up review — 2026-11-08

The four follow-up findings are fixed and tested; the earlier six remain fixed.

7. **Upload and service-control locking.** `UPLOAD_PREPARE` and
   `SERVICE_CONTROL` are now in the agent's mutation guard, with the service
   root read from the nested `service.root`. The upload carries the project
   lock token. The receiver refuses a ticket when the lock is free or belongs
   to another token, and re-checks during the stream. The engine's
   `service/control` takes the project lock for the whole change. Unit tests
   cover an upload and a service control without the lock, and an upload ticket
   after its lock is released.
8. **Lost legacy lock channel.** The plain SSH lock is split into a reader and
   a writer. A monitor marks the guard dead when the shell's channel ends, and
   `is_alive` checks that flag as well as the SSH session. Release reports
   shutdown, monitor and timeout errors instead of ignoring them. A Podman
   test killed the lock shell during a deploy: the deploy failed with
   "the connection holding the project lock was lost" and the live release
   stayed unchanged.
9. **Commands outliving the agent lock.** Every `deploy.yaml` server command
   now writes a `.shipdeck.command` lease with its shell PID and a token, and
   removes it on exit. A new deploy, rollback, cleanup or service control
   checks the lease before taking the lock and refuses while the command is
   alive; a stale lease is removed. In Podman, the agent was killed during a
   long setup command. A second deploy was refused with "a server command is
   still running for this project"; after the command ended, a third deploy
   succeeded.
10. **Failed backup publish.** `publish_backup` removes the `.part` file only
    when the final name already exists. Any other publish error keeps the
    packed part and reports its path. A unit test covers the kept part on a
    failed publish.

Final verification: `cargo fmt --all -- --check`, workspace compilation,
all tests (38 CLI, 43 agent, two agent process, 233 engine, 24 protocol, 34
TUI), strict pedantic/performance Clippy, schema regeneration,
`git diff --check`, `dist plan`, and `dist generate --check`. The existing
monorepo SSH suite passed after all lock and lease changes, including deploy,
unchanged, manager/app changes, shared data, retention, compressed backup
rollback, failed restore, setup failure, partial activation recovery, custom
PM2 backups, done warnings, and RPC controls/logs.

Remaining limits: disk-full behavior and an actual GitHub release publication
were not tested; 32-bit ARM has no release build. Upload speed still has no
measured gain over SFTP or direct SSH.
