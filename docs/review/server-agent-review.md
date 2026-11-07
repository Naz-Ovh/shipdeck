# (done) Server-agent code review

Reviewed on 2026-11-07. Scope: agent changes introduced by `3a045b7` and merged
by `ecb1d3f`, followed by a review of the uncommitted corrections on 2026-11-07.
The original findings are retained below. The second review's timeout-cleanup
finding was fixed on 2026-11-07; see Resolution. Original file locations refer
to reviewed commits.

## Result

The implementation keeps the agent separate from the engine, shares protocol
types, quotes shell arguments, checks downloaded archive hashes, and preserves
the existing deployment path after reported agent failures. Automated checks
pass. The second review confirms the directory and RPC corrections, and that
timeouts let the caller continue. Remote process termination is now handled by
the server-side `timeout`.

## Findings

### P1: temporary installation accepts an untrusted existing directory

Location: [private directory preparation](../../crates/engine/src/agent/mod.rs:188).

The fallback uses a predictable `.shipdeck-<uid>` directory under the temporary
directory. `mkdir -p` and `chmod` accept existing directories and follow a final
symlink. They do not verify ownership or establish that the location is trusted.

This matters when connecting as root and the home installation cannot run.
Another local user can pre-create the fallback directory and remain its owner,
even after root changes its mode. That owner can alter the directory contents
while the installer prepares or executes the agent. A checksum on the downloaded
archive does not protect the subsequently installed path from replacement.

A local reproducer confirmed that the exact preparation command accepts a
symlink and changes its target's permissions. A cross-user exploit was not run.

Required fix: securely create a private owned directory, reject untrusted
existing directories and symlinks, use unpredictable staging paths, and install
without a writable-by-another-user path between validation and execution.
Add tests for existing attacker-owned paths, symlinks, and concurrent installs.

### P1: optional agent installation can block normal SSH operations

Locations: [download script](../../crates/engine/src/agent/binary.rs:168) and
[connection setup](../../crates/engine/src/ssh/mod.rs:510).

Every new connection awaits agent startup before returning the session. The
curl/wget downloads have no connection or overall deadline. The 15-second hello
timeout begins only after installation and process launch. A reachable download
endpoint that stops transferring can therefore block connection setup and the
existing SSH deployment path instead of returning an optional-agent failure.

A reproducer ran the actual installer template with a deliberately stalled
downloader. It remained blocked until the test terminated it. Source inspection
confirmed there is no enclosing startup timeout.

Required fix: bound the complete optional startup operation and network
transfers, close cancelled SSH channels, and ensure interrupted installation
leaves no active partial binary. Test a stalled endpoint through real SSH and
confirm ordinary SSH operations remain available after the deadline.

### P2: some unsupported RPC requests never receive an error response

Location: [request dispatch](../../crates/agent/src/lib.rs:25).

The shared connection classifies recognised `ask/*` requests as `Incoming::Ask`.
The agent handles only `Incoming::Other` and silently skips all other variants.
Consequently a valid unsupported `ask/secret` request receives no reply, while
`agent/missing` correctly receives METHOD_NOT_FOUND. A caller awaiting that
request can remain blocked until disconnect.

Reproduced against the built agent executable: `agent/missing` replied;
`ask/secret` did not reply within one second. The source confirms it is skipped,
rather than delayed. The current engine does not normally send this request to
the agent, so this is protocol robustness rather than a normal deployment fault.

Required fix: return METHOD_NOT_FOUND for every unsupported request carrying an
ID, including classified Ask variants. Add process-level regression coverage.

## Second review: fixed finding

### P2: closing a timed-out SSH channel does not stop the remote command

Location: [timeout handling](../../crates/engine/src/ssh/mod.rs:282).

The timeout path closes the SSH channel and returns an error. It does not
establish that the remote process has terminated. The documentation and code
comments claim that nothing remains running, but a real SSH reproduction shows
that the command can continue and perform later filesystem writes.

Using the current Rust `Session::exec_raw_within` against the disposable Podman
OpenSSH server, ran `sleep 8; printf done > /tmp/shipdeck-review-timeout-marker`
with a 300 ms limit. The call returned a timeout after about 344 ms. A second
command on the same session succeeded immediately. Nine seconds later, the
marker contained `done`. The test removed that marker afterward.

An installer that outlives its reported timeout can continue downloading or
mutating installation files while fallback or a later connection proceeds.
Downloader-specific deadlines reduce its lifetime but do not provide the
claimed cancellation guarantee for the whole script or its descendants.

Required fix: implement remote cancellation for the bootstrap command and its
children, with bounded cleanup and observable completion. Do not assume a
channel-close packet terminates a non-PTY process. Add an SSH regression test
that checks a delayed side effect does not happen after cancellation. Correct
the lifecycle documentation to match the behavior actually guaranteed.

Fixed: limited commands now run under the server's `timeout -k 3 <limit>`,
which signals the command's process group; the channel close is only a
backstop. See Resolution.

## Resolution

Corrections supplied on 2026-11-07. All findings, including the second review's
cleanup finding, are fixed.

- P1 temporary directory: the fallback is a fresh `mktemp -d` folder. Before
  use it checks for a symlink, this user's ownership, and mode `0700`.
  Concurrent connects get different folders. `install_test.rs` covers a
  symlink, a world-open folder, and two temp folders.
- P1 blocking startup: one raw command gets 15 s, the install 45 s, opening the
  channel 15 s, hello 15 s, and the whole startup 90 s. `curl` and `wget` carry
  their own limits.
- P2 remote cancellation (second review): each limited command runs as
  `timeout -k 3 <limit> sh -c '<script>'` on the server. TERM goes to the
  process group, KILL follows after 3 s. The engine maps exit 124 or 143 to a
  timeout. Closing the channel is only the backstop.
- P2 RPC: every unsupported request with an id, including `ask/*`, gets
  `METHOD_NOT_FOUND`. `serve_test.rs` and `tests/process.rs` cover it.
- Layout: `lib.rs` and `mod.rs` are declarations only. The code is in
  `serve.rs`, `session.rs`, and `install.rs`.

Verification: formatting, pedantic clippy, and all workspace tests pass. On the
Podman SSH test server, forcing the home install to fail made the secure tmp
fallback run. A stalled download returned after 90 s, the release list still
worked, and no `curl` was left behind. A real SSH check of the wrapper ran
`timeout -k 3 1 sh -c "sleep 3; printf done > marker"`; it returned 124, and
the marker was absent 4 s later. The agent install still completed through the
wrapper, and a local regression test covers the same side effect. A full
four-target release build was not run.

Independent second-review verification: formatting, all 329 workspace tests,
and strict pedantic/performance Clippy passed. The real SSH check confirmed a
working optional-agent failure path, command timeout, and immediate session
reuse. It also reproduced the surviving command that became the finding above;
the Resolution's server-side `timeout` prevents it. Reproducer:
`/tmp/vscode-agent-codex-sandbox-H2LGk7/agent-recheck/src/main.rs`. It uses the
workspace's locked dependency versions. No application source was changed by
the second review, and no four-target release build was run.

## Verification and limits

Passed:

- `cargo fmt --all -- --check`.
- `cargo test --workspace`: 319 tests passed.
- `cargo clippy --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf`.
- `cargo build -p shipdeck-agent`.
- Local process and shell reproductions described above.

Reproducer and results are under
`/tmp/vscode-agent-codex-sandbox-H2LGk7/agent-review/` (`check.py`, `results.json`).

This review did not run a new Podman SSH suite, publish release artifacts, or
verify all four target binaries. The existing installer tests mostly inspect
script text; they do not establish runtime installation and recovery behavior.

There were also project-layout deviations: both agent `lib.rs` and engine agent
`mod.rs` contained implementation, whereas project rules reserve these files for
declarations and re-exports. The Resolution fixed them.
