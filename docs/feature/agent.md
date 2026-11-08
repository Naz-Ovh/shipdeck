# Server agent

Working checklist: [Agent completion plan](../../.agents/task/agent-completion.md). It records
the current scope, open decisions, upload design, and verification evidence.

## The problem

The engine used to do every server step over SSH with `sh` scripts. The steps worked, but they
read text back from a shell. Files, services and logs are sturdier as typed requests and live
events.

## The chosen design

A small program, `shipdeck-agent`, runs on the server while you are connected.

- The engine installs it the first time and starts it over SSH. The server downloads its
  published build over HTTPS. Set `SHIPDECK_AGENT_BINARY` to an explicit local Linux binary to
  upload a development build over SSH. This is not a fallback when a published download fails.
- It runs as your SSH user, opens no port, and exits when the connection ends.
- The engine and the agent speak the same JSON-RPC as the apps.
- The agent's version always matches the engine's.

The gnu build is first, for servers on glibc 2.31 (Ubuntu 20.04) or newer. Alpine and older
servers get the static musl build. `dist` builds four Linux archives; `dist plan` on this tree
lists them, and the generated release workflow is current.

## What the agent does

All of these are typed requests; no request carries a shell line.

| Area | Operation |
| --- | --- |
| Files | Hash live-release files for reuse, prepare project folders, switch `current` atomically. |
| Upload | Reserve a one-use ticket, receive a bundle on a second SSH channel, check length and SHA-256, keep partial files. |
| Unpack | Extract tar/zstd with path, symlink, mode, entry and byte limits; copy unchanged files; verify every file against the receipt; keep staging on mismatch. |
| Shared | Link `service.shared` folders into a release, creating each in `shared/` the first time. |
| Backups | Pack selected release paths natively into `backups/<id>.tar.zst`, restore one, list the paths. |
| Releases | List open releases and backups, read a release folder or its backup, read one file, drop one release or backup. |
| Services | Read systemd/PM2 state, stop, start and restart, with `sudo -n` only when the user is not root. |
| Logs | Follow journal or PM2 output and send each line as a notification; stop on request. |
| Locks | Hold one project lock per SSH session with an OS file lock and a private token. |
| Events | Append numbered events to `~/.shipdeck/state.db`; read a page after a cursor, or a project's newest page. |
| Cleanup | Remove exactly the paths `work/get` reported, only after the user approves them. |
| Setup | Guided one-time preparation: packages, project folder, and a checked sudoers rule. |

The engine keeps the old SSH commands as a fallback when the agent is not available, so servers
that cannot run it still deploy.

## Safety rules

Remote code execution is the risk to design out, not a feature to add.

- There is no generic "run this string" request. The agent accepts typed operations only.
- Every value is checked before it reaches a path or a command: names, ports, domains, service
  names.
- Paths stay under a fixed root. `..`, absolute escapes and symlink escapes are refused.
- `deploy.yaml` server commands remain the one shell exception, because their contract is a
  shell line. They stay in the engine until a typed form exists.
- Guided setup offers full sudo or only the commands shipdeck runs. It never widens access
  without that explicit choice. The sudo password travels on stdin and is never saved.
- An interrupted upload is bounded: a dead receiver cannot make the engine wait forever.
- Interrupted files stay until the user runs `shipdeck work --cleanup` or the TUI's `x`. Cleanup
  re-checks the server's current list and refuses any path it did not report.

## Concurrency and command lifetime

Every deploy, rollback, cleanup, and service control takes one project lock.
With an agent it is an OS file lock; on the plain SSH path a dedicated shell
holds `flock` and is monitored, so losing that channel fails the run instead of
silently freeing the project. Agent mutations refuse to run unless the same
connection holds the lock.

An upload ticket is tied to the lock token. The receiver refuses it when the
lock is free or belongs to another operation, and re-checks while bytes arrive.
Every `deploy.yaml` server command writes `.shipdeck.command` with its shell
PID and a token. A new operation refuses while that command is alive, so an
agent disconnect cannot let a second deploy overlap a command that is still
running. A stale lease is removed after its process is gone.

## Events and reconnect

`~/.shipdeck/state.db` stores numbered events: build, pack, upload start/verify, unpack
start/verify, setup, backup, switch start/complete, service restart, health, and the final
outcome, plus rollback events. `shipdeck events` prints them and supports `--after` for a
cursor; the TUI shows the newest ones with `work`.

A dropped connection during preparation leaves the live release unchanged. A dropped connection
during activation is reported as a failure, never as success. On reconnect, `shipdeck work`
shows the actual `current`, the service state, unfinished paths, and a warning when the last
activation event was a switch or restart with no health pass after it. It does not guess.

## Steps

| Id | Step | State |
| --- | --- | --- |
| p3a1 | Uploaded and updated on connect | built |
| p3a2 | The same messages as the engine | built |
| p3a3 | Files, natively | built |
| p3a4 | Final activation and interrupted-work inspection | built |
| p3b1 | `state.db`: every event, numbered | built |
| p3c1 | Set up a server from the app, once | built |
| p3c2 | Server facts in CLI and TUI | built |

## Verified

- Workspace formatting, compilation, strict pedantic/performance Clippy, and all tests pass.
- Podman SSH: first deploy, second deploy with reuse, rollback to an open release, rollback to a
  compressed release through native restore, shared-folder persistence, unchanged detection
  (exit 6), release listing with native reads, release contents/file reads, service stop/start,
  live logs through the agent, guided setup with command-level and full sudo access, a
  non-root deploy after setup, a permission failure before setup that left the live release
  alone, an interrupted upload that kept a partial and left the live release unchanged,
  `work` inspection and approved cleanup, and a dropped activation connection that exited 5 and
  was then inspected through `work`.
- The plain SSH path was tested by pointing `SHIPDECK_AGENT_BINARY` at a missing file: the
  deploy completed over the old shell path.
- The release agent was run in an isolated `/tmp` folder on the owner's VPS: `--version`,
  `agent/hello`, `agent/ping`, and `agent/shutdown` all answered; the folder was removed and no
  existing deployment or Shipdeck state was touched.

- Follow-up review: upload and service control need the lock; tickets are bound
  to its token; the plain SSH lock channel is monitored; command leases block
  overlap after an agent disconnect; a failed backup publish keeps its part.
  Podman killed the lock shell and the agent during a long command: the first
  deploy failed, the live release stayed unchanged, the next deploy was refused,
  and a later one succeeded.

## Open points

- **p3d2, plain deploys without the agent.** Recommended and followed: the SSH path keeps
  working exactly as before, and a missing agent never blocks it.
- **32-bit ARM.** No release build yet. The engine names the CPU and stops instead of installing
  a binary that cannot run.
- **Releases.** The agent artifacts must be in the release that carries this code. `dist plan`
  shows them; a published release still needs a version bump and the archive assets.
- **Upload speed.** Local and VPS measurements did not show the agent receiver beating SFTP or
  direct SSH. The agent adds verification and typed failure handling, not a proven speed gain.
