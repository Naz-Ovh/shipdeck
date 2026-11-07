# Server-agent code review

Reviewed on 2026-11-07. Scope: agent changes introduced by `3a045b7` and merged
by `ecb1d3f`. This is a review, not a fix. No agent source was changed.

## Result

The implementation keeps the agent separate from the engine, shares protocol
types, quotes shell arguments, checks downloaded archive hashes, and preserves
the existing deployment path after reported agent failures. Automated checks
pass. The findings below prevent an unconditional readiness approval.

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

There are also project-layout deviations: both agent `lib.rs` and engine agent
`mod.rs` contain implementation, whereas project rules reserve these files for
declarations and re-exports. Address these during the agent corrections.
