# The agent

`shipdeck-agent` is a small program the engine starts on a server. It runs
only while the engine's SSH connection is open, as the SSH user, and exits when
that connection ends. Nothing listens on a port.

This page covers p3a1 and p3a2. File work and deploy jobs come later.

## The wire

The agent speaks the same JSON-RPC 2.0 as the app protocol: one message per
line of UTF-8, on stdin and stdout. Only `crates/protocol` defines the words;
the agent does not depend on the engine.

| Method | Params | Answer |
| --- | --- | --- |
| `agent/hello` | `protocol`, `version` | `protocol`, `version`, `os`, `arch` |
| `agent/ping` | none | none |
| `agent/shutdown` | none | none; then the agent exits |

The protocol version is `1`. The engine sends `agent/hello` first. A different
version is refused with `PROTOCOL_VERSION`, and the agent exits. The agent's
version must match the engine's; both use `[workspace.package].version`.

`Connection` from the protocol crate is reused as-is. Agent methods are not in
its `Ask` or `Event` enums, so they arrive as `Incoming::Other` and the agent
matches the method name.

## Building it

The agent is its own package, `crates/agent`, so it never pulls the engine in.
It is distributed beside `shipdeck` in the same GitHub release, for four
targets:

| CPU | glibc 2.31 or newer | older glibc or musl |
| --- | --- | --- |
| x86_64 | `x86_64-unknown-linux-gnu` | `x86_64-unknown-linux-musl` |
| aarch64 | `aarch64-unknown-linux-gnu` | `aarch64-unknown-linux-musl` |

The gnu build uses the same manylinux container as `shipdeck`; the release
check allows no glibc newer than 2.31 (Ubuntu 20.04). The musl builds are
static. Archives are `.tar.gz`, named `shipdeck-agent-<target>.tar.gz`.

32-bit ARM has no build yet. The engine says so plainly instead of uploading a
binary the server cannot run.

## Choosing the build

The engine asks the server with `uname -s`, `uname -m`, `getconf
GNU_LIBC_VERSION`, and `ldd --version` (its first line only). The order is:

1. Not Linux: no agent.
2. glibc 2.31 or newer: the gnu build.
3. musl (Alpine) or an older glibc: the musl build.

These commands run without the login shell's startup files. A startup file
that prints would corrupt the protocol.

## Getting it there

The bytes never cross SSH. The engine sends a fixed install script, and the
**server** downloads its own build over HTTPS:

1. If `$HOME/.shipdeck/bin/shipdeck-agent-<version>` already runs and reports
   the right version, nothing happens.
2. Otherwise the script fetches `shipdeck-agent-<target>.tar.gz` and its
   `.sha256` file from the GitHub release for this version.
3. It checks the hash, unpacks, moves the binary in place, and sets mode
   `0700`.

The server needs `curl` or `wget`, `sha256sum`, `awk`, `tar` and `find`, plus
outbound HTTPS. When one is missing, the engine says so and the SSH deploy path
keeps working. `SHIPDECK_AGENT_BASE_URL` names another download folder, for a
mirror or a local test.

Only the current version is kept. Older `shipdeck-agent-*` files in the home
folder are removed after a successful start.

## Starting it

With the binary in place, the engine runs:

```sh
sh -c 'exec <path> 2>> <folder>/agent.log'
```

It does not use the login shell's startup files, and the process replaces
`sh`, so stdout carries only protocol lines. Diagnostics go to `agent.log`.

The engine runs `<path> --version` and compares it with its own before starting.

If the home folder does not allow running programs (`noexec`) or cannot be
written, the same download repeats under `${TMPDIR:-/tmp}/.shipdeck-<uid>/`,
mode `0700`. When neither place works, the engine says so and the SSH deploy
path keeps working.

## Lifecycle

- The engine starts the agent once per `user@host:port`, right after login.
- A failure is printed to the engine's stderr and stored as "no agent". It does
  not fail the connection or the deploy.
- `Session::close` sends `agent/shutdown` before the SSH disconnect.
- Dropping the session closes the SSH channel. The agent sees end of input and
  exits.

## Tests

- `crates/agent/src/lib_test.rs`: hello, ping, shutdown, an unknown method, bad
  params, and a foreign protocol, all over in-memory pipes.
- `crates/engine/src/agent/binary_test.rs`: glibc parsing, numeric comparison,
  gnu/musl choice, CPU names, release names, and the install script's download,
  checksum and quoting steps.
