# Server agent

Agreed rules: [Agent access and version policy](agent-version-policy.md).
Working checklist: [Agent completion plan](../../.agents/task/agent-completion.md).

## The problem

The engine used to do every server step over SSH with `sh` scripts. The steps
worked, but they read text back from a shell. Files, services and logs are
sturdier as typed requests and live events.

## The chosen design

A small program, `shipdeck-agent`, runs on the server while you are connected.

- Preparation happens on the first connection. The account logs in with its
  supplied SSH credentials and must be root or have full password-free sudo
  (`NOPASSWD: ALL`). A sudo rule limited to some commands is not supported.
- The agent is installed at `/var/lib/shipdeck/agent/v<version>/agent`. Root owns
  the executable and every parent directory. It is started through `sudo`
  with the full path.
- It opens no port, runs only while the SSH connection is open, and exits when
  the connection ends.
- The engine and the agent speak the same JSON-RPC as the apps.
- The agent's major version must match the engine's. A different minor or patch
  version within the same major is allowed. A newer agent major refuses the
  connection.
- If the engine major is newer, older agents are removed and the matching
  release is installed before the handshake.
- Set `SHIPDECK_AGENT_BINARY` to an explicit local Linux binary to use a
  development build. The upload is copied and checked by root before it runs.
  It is not a download fallback.

The gnu build is first, for servers on glibc 2.31 (Ubuntu 20.04) or newer.
Alpine and older servers get the static musl build. `dist` builds four Linux
archives; `dist plan` on this tree lists them, and the generated release
workflow is current.

## Where the agent lives

Everything the agent keeps on a server is in one folder, `/var/lib/shipdeck`:

```
/var/lib/shipdeck/                 root-owned; only the agent writes here
├── agent/v<version>/agent         the program, launched by full path
├── agent/.install.lock            coordinates simultaneous installs
├── agent.log                      the agent's own errors, written by the agent
└── users/<ssh user>/state.db      that user's events, folder mode 700
```

Nothing shipdeck writes lands in the SSH user's home.

Why this folder (decided 2026-11-11):

- **One owner.** The agent runs as root. Its files must be writable by root
  only. If a normal user owned the folder, a process running as that user could
  replace `state.db` with a link to a system file, and the root agent would
  write into it. A root-owned folder written only by the agent rules that out.
- **One place.** The program, its log and its data sit together, so an admin
  finds, checks and removes shipdeck in one step.
- **The standard spot.** `/var/lib/<app>` is where Linux keeps data a program
  changes as it runs (FHS). `/usr` is for package-manager files and is
  read-only on some systems.
- **Per-user events.** `users/<ssh user>/` keeps each user's history apart,
  as the home folder did before.
- **The agent writes its own log.** Before, the SSH user's shell opened the
  log, so the log had to be in a folder that user could write.

The cost we accepted: a server that mounts `/var` with `noexec` cannot run the
agent from here. The install then fails, and the agent does not start. Such a
server needs `/var` mounted without `noexec`.

Before this, the program was in `/opt/shipdeck/agent`, and the events and log
were in `~/.shipdeck/state.db` and `~/.shipdeck-agent.log`. Shipdeck does not
move old events to the new place.

## Installation

The engine lists valid installations under `/var/lib/shipdeck/agent`. A directory
counts only when its executable and every parent are root-owned, not writable
by other users, and the binary reports the version in the directory name.

The installer runs as root inside one lock. It downloads the versioned archive
and its `.sha256` from GitHub, checks the checksum, archive paths and entry
types, and checks the binary's printed version. It publishes a complete
version directory with one rename and removes older version directories.

A private staging folder holds the download and unpacked files. A failed or
interrupted install never publishes a valid version directory. Two simultaneous
connections coordinate through the lock; the second reuses the first result.

## What the agent does

All of these are typed requests; no request carries a shell line.

| Area | Operation |
| --- | --- |
| Files | Hash live-release files for reuse, prepare project folders, switch `current` atomically. |
| Upload | Reserve a one-use ticket, receive a bundle on a second SSH channel, check length and SHA-256, keep partial files. |
| Unpack | Extract tar/zstd with path, symlink, mode, entry and byte limits; copy unchanged files; verify every file against the receipt; keep staging on mismatch. |
| Shared | Link the `shared` folders into a release, creating each in `shared/` the first time. |
| Backups | Pack selected release paths natively into `backups/<id>.tar.zst`, restore one, list the paths. |
| Releases | List open releases and backups, read a release folder or its backup, read one file, drop one release or backup. |
| Services | Read systemd/PM2 state, stop, start and restart. Systemd runs as root. PM2 runs as the service user. |
| Logs | Follow journal or PM2 output and send each line as a notification; stop on request. |
| Locks | Hold one project lock per SSH session with an OS file lock and a private token. |
| Events | Append numbered events to the SSH user's `/var/lib/shipdeck/users/<user>/state.db`; read a page after a cursor, or a project's newest page. |
| Cleanup | Remove exactly the paths `work/get` reported, only after the user approves them. |

The engine still contains the older SSH command paths, but a connection now
fails when agent preparation fails. A plain SSH connection is not used to bypass
an incompatible or broken agent.

## Safety rules

Remote code execution is the risk to design out, not a feature to add.

- There is no generic "run this string" request. The agent accepts typed
  operations only.
- Every value is checked before it reaches a path or a command: names, ports,
  domains, service names.
- Paths stay under a fixed root. `..`, absolute escapes and symlink escapes are
  refused.
- The installer never executes a user-writable development binary as root.
- App and PM2 commands carry an explicit service user. They never inherit the
  agent's root identity.
- Deployment files keep the SSH user's ownership. Existing app and service
  users are preserved.
- App archives must remain inside their intended release. The agent keeps the
  path, archive, symlink, mode, entry and size checks under root.
- Your server commands from the deploy file remain the one shell exception,
  because their contract is a shell line. They run as the deploy user.
- shipdeck never asks for a sudo password and never changes the server's sudo
  rules. The account must already be root or have password-free sudo.
- An interrupted upload is bounded: a dead receiver cannot make the engine wait
  forever.
- Interrupted files stay until the user runs `shipdeck work --cleanup` or the
  TUI's `x`. Cleanup re-checks the server's current list and refuses any path it
  did not report.

## Concurrency and command lifetime

Every deploy, rollback, cleanup, and service control takes one project lock.
The lock is an OS file lock held by the connected agent. Agent mutations refuse
to run unless the same connection holds the lock.

An upload ticket is tied to the lock token. The receiver refuses it when the
lock is free or belongs to another operation, and re-checks while bytes arrive.
Every server command from the deploy file writes `.shipdeck.command` with its shell
PID and a token. A new operation refuses while that command is alive, so an
agent disconnect cannot let a second deploy overlap a command that is still
running. A stale lease is removed after its process is gone.

## Events and reconnect

Each SSH user's `/var/lib/shipdeck/users/<user>/state.db` stores numbered events: build, pack,
upload start/verify, unpack start/verify, setup, backup, switch start/complete,
service restart, health, and the final outcome, plus rollback events.
`shipdeck events` prints them and supports `--after` for a cursor; the TUI shows
the newest ones with `work`.

A dropped connection during preparation leaves the live release unchanged. A
dropped connection during activation is reported as a failure, never as
success. On reconnect, `shipdeck work` shows the actual `current`, the service
state, unfinished paths, and a warning when the last activation event was a
switch or restart with no health pass after it. It does not guess.

## Steps

| Id | Step | State |
| --- | --- | --- |
| p3a1 | Automatic privileged install and update on connect | built |
| p3a2 | The same messages as the engine | built |
| p3a3 | Files, natively | built |
| p3a4 | Final activation and interrupted-work inspection | built |
| p3b1 | `state.db`: every event, numbered | built |
| p3c1 | Guided setup from the app | removed on 2026-11-12 |
| p3c2 | Server facts in CLI and TUI | built |
| p3d1 | Agent root access and execution identity | built |

## Verified

- Workspace formatting, compilation, strict pedantic/performance Clippy, and all
  tests pass.
- Podman: first agent install with no installation, reconnection, same-major
  `0.6.9`, newer-major refusal, older-directory cleanup, two simultaneous
  installs, interrupted download, bad checksum, wrong binary version, failed
  handshake, root ownership, replacement attempts by ordinary users, development
  upload, missing sudo, PM2 deploy, systemd deploy, a setup command running as
  `shipdeploy`, and non-interactive CLI use.
- The CLI-major-greater decision and its cleanup are covered by unit tests.
- Earlier Podman coverage still applies: repeat deploys, rollback, shared data,
  unchanged detection, native release reads, service controls, live logs,
  approved cleanup, and reconnect inspection.

## Open points

- **Plain SSH path.** It is no longer used to bypass an agent failure. The old
  code remains for the legacy internal paths but a connection requires the
  agent.
- **32-bit ARM.** No release build yet. The engine names the CPU and stops
  instead of installing a binary that cannot run.
- **Releases.** The agent artifacts must be in the release that carries this
  code. This change requires a major release. The owner must choose and publish
  the version.
- **Upload speed.** Local and VPS measurements did not show the agent receiver
  beating SFTP or direct SSH. The agent adds verification and typed failure
  handling, not a proven speed gain.
