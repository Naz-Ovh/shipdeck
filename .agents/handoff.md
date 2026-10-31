# Handoff

Where the last session stopped (2026-10-31). Read [README.md](README.md) first; steps and status are in
[the plan](../docs/project-progress.html).

## State

- Branch `feat/gui`, from `b5dff48`. **Nothing is committed.** Commit only when the owner asks.
- `docs/` root holds only `plan.md`, `project-progress.html`, `visual/` and `website/` (the owner wants it kept
  clean). New, untracked:
  - `docs/project-progress.html`: the plan (phases 0 to 9). Phase 0 is what shipdeck does today; five of its
    items are built but not released (`p0b3`, `p0c4`, `p0c5`, `p0e3`, `p0f2`).
  - `docs/visual/Plan/spec.html`: the spec, a pan-and-zoom canvas (`p1a1`, built, waiting for the owner's look).
    `docs/visual/diagram/` and `docs/visual/prototype/` are empty folders the owner made; the desktop mockup
    (`p2a1`) presumably goes in `prototype/` (ask).
- Merged into the spec and deleted on 2026-10-31: the untracked `PLAN_desktop.html`, `PLAN_roadmap.html`,
  `architecture.html`, `agent-tradeoffs.html`, `scope.html`, `diagrams.html`, and the committed
  `PLAN_config-spec.html`, `PLAN_packaging.html`, `PLAN_status-logs.html` (deleted, not staged). Their open items
  became `p1f1`, `p1f2` (status and logs) and `p9b1`–`p9b4` (install channels).

## Decided this session

All of it is in the plan; the short version:

- One headless process, `shipdeck engine`, does all the work. The CLI, the TUI, the Avalonia desktop app and later
  a web dashboard are its clients, over JSON-RPC 2.0 on stdin/stdout. Frontends depend only on a protocol crate.
- Monorepo: `apps/` (cli, desktop) + `crates/`; `bin/shipdeck` → `apps/cli` first. The website stays in
  `docs/website` and moves to TanStack Start on its own track.
- `shipdeck-agent`: uploaded on connect at the engine's version, runs as the SSH user while connected, sudo when
  needed, native file ops; the server half of a deploy runs under `systemd-run`; events go to
  `~/.shipdeck/state.db` with sequence numbers for resync. `shipdeck-keepup` (always on, unix socket) comes later.
- Then domains with nginx (not Caddy), databases, ready-made apps, mail.
- The five scope decisions: offline cache in the engine; the TUI/CLI run the engine as a separate process; the
  engine shows database tables over an SSH tunnel; the web dashboard is an always-on engine on a home server;
  keepup runs as root only if the server's owner chooses.
- Still open (status `waiting`): `p2d1`–`p2d4` (desktop) and `p3d1`, `p3d2` (agent).

## Owner preferences

- Chat replies in easy English; Bengali only when the owner asks for it.
- Pages for the owner (plan, spec) in very easy English: short sentences, common words, careful punctuation.
- The owner loves the look of the spec and progress pages: dotted canvas, tinted sections, white frames, yellow
  sticky notes. Reuse it for new visual pages.

## Next

1. The owner looks at the spec (`p1a1`) and says yes or what to change.
2. `p2a1`: the clickable HTML mockup of the desktop app.
3. `p1a2`: the move to `apps/` + `crates/`, then `p1b1`, the protocol crate.

## Gotchas

- Canvas pages: give the viewport `overflow: clip`. With only `overflow: hidden`, a `#id` in the URL scrolls the
  viewport and the pan/zoom maths goes wrong. Wait for the web font (`document.fonts.load`) before the first fit,
  and fit again on `ResizeObserver` until the reader moves.
- Screenshots of a page: `msedge --headless=new --disable-gpu --hide-scrollbars --window-size=W,H
  --virtual-time-budget=5000 --screenshot=<file> file:///<path>#<id>`; add `--force-dark-mode` for dark.
