# Handoff

Where the last session stopped (2026-11-06). Read [README.md](README.md) first; steps and status are in
[the plan](../docs/project-progress.html).

## State

- One branch: `main`. The owner works alone. Everything is committed; **not pushed**: the owner pushes in
  one go, later.
- Desktop work stays deferred (owner, 2026-11-05). The CLI and the headless crate on the new architecture come
  first.
- Done this session:
  - `p1a2`: `bin/shipdeck` moved to `apps/cli` (git history kept), empty `bin/` removed. Every doc path,
    the website install page and the tracker updated.
  - The pedantic clippy rule now holds over the whole workspace. The old code was fixed by hand (no
    `#[allow]`, no `#[expect]`); `lefthook.yml` runs `-D warnings -W clippy::pedantic -W clippy::perf` on
    push, so new code follows it.
  - `p1b1`, `p1b2`: `crates/protocol` holds every v1 message as a Rust type, `protocol.schema.json`, and a
    client over any async read/write pair. `just protocol` regenerates the schema; a test fails when it is
    stale.
- `AGENTS.md` holds only the rules for every session; the detail is in `docs/code/`. `docs/` root holds only
  `plan.md`, `project-progress.html`, `visual/`, `website/`, `code/` and `feature/`.

## Decided this session

- The protocol crate is the contract: JSON-RPC 2.0, one message per line, `snake_case`, variants as one flat
  object with a `kind`. Names are only added, never renamed; `initialize` carries protocol `1` and
  capabilities. Errors: `-327xx` are JSON-RPC's own, shipdeck's start at `-32001`.
- Version 1 carries what the TUI does today: `workspace/open`, `status/get`, `connection/close`,
  `service/control`, `releases/list`, `release/contents`, `release/read_file`, `logs/follow`, `logs/stop`,
  `deploy/start`, `rollback/start`, the `op/*` and `logs/*` events, and the four `ask/*` questions.
  `status/get` answers `dir_exists`, `current` and `state` only; the richer status line is `p1f1`.
- The crate is transport-agnostic. `Client::start(read, write)` takes pipes in the app and an in-memory pipe
  in tests; the engine's process plumbing lands with `p1c1`.

## Next

1. `p1c1`: `shipdeck engine`, the process that reads requests and writes events. Open in the spec: whether
   its message loop lives inside `crates/engine` or in a crate of its own. Decide it when building, with the
   owner.
2. Then `p1c2` (safe when its app closes), and `p1d1`/`p1d2` (the CLI and the TUI use the engine).
3. Owner still to look at: the spec (`p1a1`) and the desktop mockup (`p2a1`). Both stay `built`.
4. Still open: `p1c6` (settings backup), `p1c7` (idle disconnect), `p2d1`–`p2d4` (desktop), `p3d1`, `p3d2`
   (agent).

## Gotchas

- Canvas pages: give the viewport `overflow: clip`. With only `overflow: hidden`, a `#id` in the URL scrolls
  the viewport and the pan/zoom maths goes wrong. Wait for the web font (`document.fonts.load`) before the
  first fit, and fit again on `ResizeObserver` until the reader moves.
- Screenshots of a page: `msedge --headless=new --disable-gpu --hide-scrollbars --window-size=W,H
  --virtual-time-budget=5000 --screenshot=<file> file:///<path>#<id>`; add `--force-dark-mode` for dark.
- Two schemas, two recipes: `just schema` (deploy.yaml) and `just protocol` (the messages). Both are checked
  by tests, so a doc comment on a config field or a message type means regenerating before the tests pass.
- The protocol client's reading task holds the event sender. `Shared::close` must drop it, or `recv` never
  returns `None`. A test covers that (`the_stream_ending_is_visible`).
- `tokio::io::duplex` drops the pipe as soon as both halves are dropped; keep the engine half alive in a
  test while the call is in flight.
