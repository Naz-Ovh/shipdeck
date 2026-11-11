# The agent

`shipdeck-agent` is a small program the engine starts on a Linux server. It
runs only while the engine's SSH connection is open, and exits when that
connection ends. Nothing listens on a port.

## The wire

The agent speaks the same JSON-RPC 2.0 as the app protocol: one message per
line of UTF-8, on stdin and stdout. Only `crates/protocol` defines the words;
the agent does not depend on the engine.

| Method | Params | Answer |
| --- | --- | --- |
| `agent/hello` | `protocol`, `version` | `protocol`, `version`, `os`, `arch` |
| `agent/ping` | none | none |
| `agent/shutdown` | none | none; then the agent exits |

The protocol version is `3`. The engine sends `agent/hello` first. A different
protocol is refused with `PROTOCOL_VERSION`, and the agent exits. The agent's
major version must match the engine's; a different minor or patch within the
same major is compatible.

`Connection` from the protocol crate is reused as-is. Agent methods are not in
its `Ask` or `Event` enums, so they arrive as `Incoming::Other` and the agent
matches the method name.

Every unsupported request that carries an id gets `METHOD_NOT_FOUND`. That
includes a recognised `ask/*` request, which the shared connection classifies
as `Incoming::Ask`. Leaving it silent would block the caller until the
connection closes.

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

## Privilege and version checks

On every new SSH connection, before any operational request:

1. The engine checks that the account is root or can run `sudo -n -- sh -c
   'id -u'`. Failure stops the connection with the account and the missing
   permission.
2. It lists `/var/lib/shipdeck/agent/v*`. A directory counts only when every parent
   directory and the executable are root-owned, not writable by group or other,
   and the executable reports the version in its directory name.
3. It applies the major-version rules in `crates/engine/src/agent/version.rs`.
   Same major uses the newest valid installation. A newer agent major refuses
   the connection. A newer engine major clears older installations and installs
   the engine's release.
4. It starts the agent with the full path through `sudo`, sends `agent/hello`,
   and checks the protocol, major version, and exact installed label.

A failure stops the connection. The engine does not run operations through an
agent that failed installation or handshake.

## Installing it

The bytes never cross SSH for a published release. The engine sends a fixed
install script, and the server downloads its own build over HTTPS:

1. Root takes `/var/lib/shipdeck/agent/.install.lock` with `flock`, or an atomic
   `.install.lock.d` directory with stale-owner detection when `flock` is
   missing.
2. A private staging directory under `/var/lib/shipdeck/agent` receives the archive
   and its `.sha256`.
3. The script checks the checksum, refuses absolute or parent-escaping paths,
   refuses links and special files, and unpacks into staging.
4. It finds exactly one `shipdeck-agent`, installs it root-owned and `0755`,
   checks its printed version, and renames the complete version directory into
   place.
5. It removes every other `v*` directory.

An interrupted attempt leaves staging or no directory, never a valid version
directory. Two simultaneous connections wait on the lock; the second reuses the
published result after re-checking it.

`SHIPDECK_AGENT_BASE_URL` names another download folder, for a mirror or a local
test. The server needs `curl` or `wget`, `sha256sum`, `awk`, `tar`, `find`,
`install`, `stat`, `chown`, `chmod`, `mktemp`, and either `flock` or `mkdir`.

## Development uploads

`SHIPDECK_AGENT_BINARY` selects an explicit local Linux binary. The engine
checks its ELF format and CPU, uploads it to a private user folder, and computes
its SHA-256. Root copies it once into the private staging directory, checks the
copied bytes and printed version, and runs only the root-owned copy. It is not
a fallback when a published download fails.

## Starting it

For a non-root account the engine runs this through the SSH channel:

```sh
sh -c 'exec sudo -n -- env SHIPDECK_OPERATOR=<user> <path>'
```

For root it drops the `sudo` prefix. The process replaces `sh`, so stdout
carries only protocol lines. The agent writes its errors to stderr and to
`/var/lib/shipdeck/agent.log` itself (`place.rs`); nothing goes to the user's home.

The upload receiver on the second SSH channel uses the same `sudo` and
identity. It does not run as the SSH user.

## Execution identity

The agent runs as root. It never lets app commands inherit that identity:

- `AgentService.user` carries the service user.
- PM2 commands run through `runuser -u <user> -- env -i` with the user's home,
  login `PATH`, `PM2_HOME`, and the service environment.
- Systemd unit installation and control stay root work.
- New project, release, backup, and shared directories are given back to the
  SSH user. Existing app and service ownership is preserved.
- Events go to `/var/lib/shipdeck/users/<SHIPDECK_OPERATOR>/state.db`. Each
  folder on the way is made mode 700 and refused if it is a link. Why this
  place: docs/feature/agent.md, "Where the agent lives".

## Limits

| Step | Limit |
| --- | --- |
| One raw command (`uname`, `home`, probe) | 15 s |
| The install script, download included | 120 s |
| Opening the agent's channel | 15 s |
| `agent/hello` | 15 s |
| The whole startup | 180 s |

Each limited command runs as `timeout -k 3 <limit> sh -c '<script>'` on the
server. `timeout` sends TERM to the command's process group, then KILL after
3 s, so the command and its children stop. Closing the SSH channel is only the
backstop: a channel close does not stop a non-PTY process. The server needs
`timeout` for this.

The install script also bounds its own transfers: `curl --connect-timeout 10
--max-time 45`, and `wget --timeout=15 --tries=2`. After the deadline the engine
logs why and fails the connection.

## Lifecycle

- The engine prepares and starts the agent once per `user@host:port`, right
  after login.
- A failure ends the connection. There is no operation without a prepared
  agent.
- `Session::close` sends `agent/shutdown` before the SSH disconnect.
- Dropping the session closes the SSH channel. The agent sees end of input and
  exits.

## Tests

- Unit tests cover version parsing and choices, install script order and
  integrity, ownership checks, launch and receiver commands, development upload
  staging, PM2 identity parsing, and file ownership helpers.
- Podman integration tests cover the installed behavior listed in
  [the feature doc](../feature/agent.md).
