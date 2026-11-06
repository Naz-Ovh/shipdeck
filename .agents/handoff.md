# Handoff

Where the last session stopped (2026-11-06). Read [README.md](README.md) first; steps and status are in
[the plan](../docs/project-progress.html).

## State

- One branch: `main`. The owner works alone. Everything is committed; **not pushed**: the owner pushes in
  one go, later.
- Done this session:
  - `p1c1`: `shipdeck engine`, the process apps talk to (`crates/engine/src/rpc`), with tests.
  - `p1d2`: the TUI is a client of that engine. `crates/tui/src/pump.rs` turns the pipe into the three
    channels the screens already read; `convert.rs` puts the answers into the engine types they draw. Every
    action is a request, every question a dialog that answers by message id. `apps/cli/src/run_tui` spawns the
    child and opens the folder first. Nothing calls `deploy` or `remote` in-process any more.
  - `p1d1`: `shipdeck deploy`, `rollback` and `releases` start that engine as a child and print what it
    reports. `apps/cli/src/console` holds `Console`: it sends the requests, answers the questions
    from the flags, and maps the outcome to the exit code. `apps/cli/src/engine.rs` holds the
    child: `serve()` is the process, `Engine::start` is the app side.
- The layout, settled by the owner: **`shipdeck` is one command, always.** `apps/` holds the programs
  people run — only `apps/cli` today, `apps/desktop` later — and `crates/` holds the code that command is
  built from: `engine`, `protocol`, `tui`. The TUI is a crate because it is not a separate command.
  Splitting crates is allowed to keep recompilation small; splitting the command is not.
- The `shipdeck-headless` crate is gone: its code is `apps/cli/src/console/`, because the console commands
  are the CLI.
- Earlier this week: `p1a2` (layout), the pedantic clippy rule, `p1b1`/`p1b2` (the protocol crate).

## Decided this session

- Version 1 grew five small things, so a screen can show what the console printed before: `workspace/open`
  takes `no_shell_env`; a project carries its `port` and its `dir`; `deploy/start` takes `on_failure`, which
  overrides `pipeline.failed.rollback` (nothing else could: a config that says `auto` never asks);
  `ReleaseName` carries `built_at`; and the `steps` in `deploy/start`'s answer are the names a screen shows
  ("Preflight"), not the keys, because every app needs the label and none needs the key.
- The app answers the engine's questions, not the other way round. `--allow-unchanged` and
  `--trust-new-host` needed no new field: they answer `ask/unchanged` and `ask/trust_host`.
- The CLI checks every `deploy.yaml` itself before it starts the engine. The engine reports the same problems
  as line/column/message, which cannot draw the caret block the console has always printed.
- The engine runs in its own process group, so the terminal's Ctrl-C stays with the app: the app decides, and
  its death closes the pipe, which ends the engine.
- `crates/tui` and `apps/cli/src/console` still use `shipdeck-engine` for types and `human` (bytes,
  durations), and the TUI reads the project's own `deploy.yaml` through it for the description panes. The
  protocol is their other shipdeck edge. If the isolation matters more than the shared formatter or the
  shared config reader, those have to move; ask the owner first.

## Next

1. `p1c2`: safe when the app closes. Today `shutdown` answers, waits for the requests it already took, closes
   the pool and exits; a deploy that is running dies with the process.
2. Owner still to look at: the spec (`p1a1`) and the desktop mockup (`p2a1`). Both stay `built`.
3. Still open: `p1c6` (settings backup), `p1c7` (idle disconnect), `p2d1`–`p2d4` (desktop), `p3d1`, `p3d2`
   (agent).

## Open threads

- Windows is untested: the engine's own process group (`CREATE_NEW_PROCESS_GROUP`) and the
  `SHIPDECK_GITSHELL_PATH` pass-through to the child. Both are `#[cfg(windows)]` and read carefully; whoever
  has a Windows box should try `shipdeck deploy` and Ctrl-C.
- Ctrl-C with two processes: the engine no longer dies first, so the first Ctrl-C only warns, as the manual
  says. `p1c2` decides what a cut-off engine does to a deploy.
- The desktop app will want the rest of what the TUI shows per project (the service name and the manager) in
  `DeployableInfo`; add it with that step.

## Gotchas

- The engine's stdin is read through `tokio::io::stdin()`, whose blocking thread a runtime shutdown waits
  for. `main` therefore calls `std::process::exit(0)` after `engine::serve()` returns; without it the engine
  outlives its protocol and the parent's `child.wait()` waits for ever. Everything queued is flushed before
  that point.
- Canvas pages: give the viewport `overflow: clip`; with only `overflow: hidden`, a `#id` in the URL scrolls
  the viewport and the pan/zoom maths goes wrong. Wait for the web font before the first fit.
- Screenshots of a page: `msedge --headless=new --disable-gpu --hide-scrollbars --window-size=W,H
  --virtual-time-budget=5000 --screenshot=<file> file:///<path>#<id>`; add `--force-dark-mode` for dark.
- Two schemas, two recipes: `just schema` (deploy.yaml) and `just protocol` (the messages). Both are checked
  by tests, so a doc comment on a config field or a message type means regenerating before the tests pass.
- `tokio::io::duplex` drops the pipe as soon as both halves are dropped; keep the engine half alive in a test
  while the call is in flight. `apps/cli/src/console/run_test.rs` has a fake engine worth copying.
- `/tmp` is not shared between two `bash` calls in this environment; use `.tmp/` under the repo for a manual
  run, and `std::env::temp_dir()` inside a test process.
- Timing `{ printf ...; sleep 6; } | shipdeck engine` measures the pipeline, not the engine. Use
  `timeout 3` on the right-hand side to prove it exits on its own.
