# Handoff

Where the last session stopped (2026-11-06). Read [README.md](README.md) first; steps and status are in
[the plan](../docs/project-progress.html).

## State

- One branch: `main`. The owner works alone. Everything is committed; **not pushed**: the owner pushes in
  one go, later.
- Desktop work stays deferred (owner, 2026-11-05). The CLI and the headless crate on the new architecture come
  first.
- Done this session:
  - `p1c1`: `shipdeck engine`, the process apps talk to. Code in `crates/engine/src/rpc/`: `run` (the loop),
    `session` (workspace, the one running operation, live logs), `convert` (inner types onto the wire), `pump`
    (op events and the four asks), and one file per method group. The subcommand is in `apps/cli`.
  - `crates/protocol`: `Client` became `Connection`, because both ends are the same thing. `handle()` gives the
    cloneable sending half (events, answers, asks), `answer_error` sends a `wire::code` number, and `flush`
    waits until what was queued is on the stream.
  - `crates/protocol`'s loop now ends reading and writing apart: answers already queued still go out after
    the app closes its side. Before this, `printf ... | shipdeck engine` lost every answer.
- Earlier this week: `p1a2` (layout), the pedantic clippy rule over the whole workspace, `p1b1`/`p1b2` (the
  protocol crate).

## Decided this session

- The engine is a library module in `crates/engine`, not a crate of its own: it reuses `config`, `deploy` and
  `remote`, and a crate would add an edge for nothing. The process is `shipdeck engine`, in `apps/cli`.
- No `server` anywhere: both ends of the pipe are `Connection`s, and the engine's half is a `Handle`.
- One request, one task; the app matches ids. `initialize` and `workspace/open` are answered in the loop
  itself, so every later message sees what they set; the rest run in parallel and answer as they finish.
- `logs/follow` and `deploy/start` answer before their first event: the app needs the view or the operation
  first.
- `initialize` lists no capabilities: nothing is optional in version 1.

## Next

1. `p1d2`: the TUI starts `shipdeck engine` as a child process and moves its screens onto the protocol. Its
   shared part comes first: spawn, `initialize`, `workspace/open`, then route `Incoming`. Then one screen at a
   time.
2. `p1c2`: safe when the app closes. Today `shutdown` waits for the request tasks, closes the pool and exits;
   a deploy that is running dies with the process.
3. `p1d1` after that: the CLI and the headless crate use the engine.
4. Owner still to look at: the spec (`p1a1`) and the desktop mockup (`p2a1`). Both stay `built`.
5. Still open: `p1c6` (settings backup), `p1c7` (idle disconnect), `p2d1`–`p2d4` (desktop), `p3d1`, `p3d2`
   (agent).

## Open threads

- `--no-shell-env` has no field in `workspace/open`, so when the TUI moves over (`p1d2`) that flag must become
  one, or the engine cannot obey it. Adding a field is compatible; do it with that step, not before.
- The desktop app's server screen will want the richer status line (`p1f1`); `status/get` carries
  `dir_exists`, `current` and `state` only.

## Gotchas

- Canvas pages: give the viewport `overflow: clip`. With only `overflow: hidden`, a `#id` in the URL scrolls
  the viewport and the pan/zoom maths goes wrong. Wait for the web font (`document.fonts.load`) before the
  first fit, and fit again on `ResizeObserver` until the reader moves.
- Screenshots of a page: `msedge --headless=new --disable-gpu --hide-scrollbars --window-size=W,H
  --virtual-time-budget=5000 --screenshot=<file> file:///<path>#<id>`; add `--force-dark-mode` for dark.
- Two schemas, two recipes: `just schema` (deploy.yaml) and `just protocol` (the messages). Both are checked
  by tests, so a doc comment on a config field or a message type means regenerating before the tests pass.
- The connection's reading task holds the event sender. `Shared::close` must drop it, or `recv` never returns
  `None`. A test covers that (`the_stream_ending_is_visible`).
- `tokio::io::duplex` drops the pipe as soon as both halves are dropped; keep the engine half alive in a test
  while the call is in flight.
- `tokio::io::stdin()` closes only its own reading loop now. Anything queued for the app must be followed by
  `Connection::flush` before the process returns, or it never reaches stdout. Both have tests in
  `crates/protocol/src/connection_test.rs`.
- `/tmp` is not shared between two `bash` calls in this environment; use `.tmp/` under the repo for a manual
  engine run, and `std::env::temp_dir()` inside a test process.
