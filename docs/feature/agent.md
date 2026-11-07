# Server agent

## The problem

Today the engine does every server step over SSH with `sh` scripts. The steps
work, but they read text back from a shell. Files, services and logs would be
sturdier as typed requests and live events.

## The chosen design

A small program, `shipdeck-agent`, runs on the server while you are connected.

- The engine installs it the first time and starts it over SSH. The server
  downloads its own build over HTTPS; the bytes do not cross SSH.
- It runs as your SSH user. It opens no port.
- It exits when the connection ends.
- The engine and the agent speak the same JSON-RPC as the apps.
- The agent's version always matches the engine's.

The gnu build is first, for servers on glibc 2.31 (Ubuntu 20.04) or newer.
Alpine and older servers get the static musl build.

## Safety rules

Remote code execution is the risk to design out, not a feature to add.

- There is no generic "run this string" request. The agent accepts typed
  operations only.
- If a program must run, the request carries the program, its argument list, the
  working folder and an environment map. Never a shell line.
- Every value is checked before it reaches a path or a command: names, ports,
  domains, service names.
- Paths stay under a fixed root. `..`, absolute escapes and symlink escapes are
  refused.
- Root work uses a fixed list of sudo commands, as today.
- `deploy.yaml` server commands remain the one shell exception, because their
  contract is a shell line. They stay in the engine until a typed form exists.
- Nginx files will be written and checked by typed operations, then reloaded by
  a fixed command. No shell interpolation.

## Why not the other two ways

- **No agent:** shell scripts are hard to grow. Every new feature becomes
  another string and another parse.
- **Always-on root agent:** it must be installed, upgraded and trusted on every
  server, and it listens all the time. The on-demand agent keeps today's good
  parts: nothing to install, nothing running when you leave, SSH as the only
  door.

An always-on helper is still possible later, as `keepup`, a separate program
that talks to the agent over a unix socket.

## Steps

| Id | Step | State |
| --- | --- | --- |
| p3a1 | Uploaded and updated on connect | built |
| p3a2 | The same messages as the engine | built |

Later steps: p3a3 (file work in Rust), p3a4 (`systemd-run` jobs that survive a
closed laptop), p3b1 (`state.db`).

## Open points

- **p3d1, how the agent gets root.** Not needed yet: this step does no root
  work. Recommended later: sudo, asking once per connection.
- **p3d2, plain deploys without the agent.** Recommended and followed: the SSH
  path keeps working exactly as before, and a missing agent never blocks it.
- **32-bit ARM.** No release build yet. The engine names the CPU and stops
  instead of installing a binary that cannot run.
- **Releases.** The agent artifacts must be in the release that carries this
  code. Until then, point `SHIPDECK_AGENT_BASE_URL` at a folder that holds the
  archive and its `.sha256`.
