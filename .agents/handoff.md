# Handoff

Where the last session stopped (2026-10-31). Read [README.md](README.md) first; steps and status are in
[the plan](../docs/project-progress.html).

## State

- Branch `feat/gui`, from `b5dff48`. The tracker and spec are committed (`cced8f3`, `874bb03`). Not committed yet:
  the desktop mockup, the tracker changes for `p2a1`/`p1c5`, `.gitignore` (the owner un-ignores the
  `easy-english` skill), `.agents/skills/easy-english/`, the relative link `.claude/skills/easy-english` and
  `skills-lock.json` (the owner's skills installer; ask before adding it). Commit only when the owner asks.
- `docs/` root holds only `plan.md`, `project-progress.html`, `visual/`, `website/`, `code/` (code guides) and
  `feature/` (feature plans); the owner wants it kept clean. New, untracked:
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

- Chat replies in easy English and quick to read; follow `.agents/skills/easy-english/SKILL.md`. Bengali only
  when the owner asks for it.
- Pages for the owner (plan, spec) in very easy English: short sentences, common words, careful punctuation.
- The owner loves the look of the spec and progress pages: dotted canvas, tinted sections, white frames, yellow
  sticky notes. Reuse it for new visual pages.

## Next

1. The owner clicks through `docs/visual/prototype/desktop.html` (`p2a1`, second take, built) and says yes or
   what to change. The owner rejected the first take (two ways in, plain dev-tool look). Now: everything starts
   from a server; home lists servers; opening one runs a connect screen, then the server's sidebar appears
   (Overview, Apps, Deployments, Logs, Files; Domains/Databases/Ready-made apps/Mail as "Soon"; Access, Server
   settings). Guided add-server and add-app flows. Openship (screenshots in `docs/refs/screenshots/openship/`)
   is the reference for look, not a copy. The owner wants a modern *native* feel, not a web page: arrow cursor,
   no selection on controls, right-click menus, shortcuts, thin scrollbars, real window buttons.
   Icons are inlined as an SVG sprite at the top of the file: Lucide (`lucide-static@0.460.0`) as `#i-<name>`
   and Simple Icons (`simple-icons@13.21.0`) as `#b-<name>`. To add one, copy its SVG body into the sprite.
   The first take is not kept. Several servers stay connected at once (`p1c7`): leaving a server starts its
   idle clock, unused for `S.idleMs` (Settings: 5/10/30 min/never; the mockup bar has a 20 s switch to try
   it) it disconnects; a running deploy keeps it connected. The owner disliked gradient buttons with glows
   ("sloppy"): primary buttons are solid, the sidebar's "Add an app" is outlined. Only the logo keeps a gradient.
   The owner's rule (2026-10-31, in `p3c1`): after the one-time server setup, nobody runs a command on the
   server, not even to add an app. The mockup's add-server flow has a Prepare step (root asked once); the
   old per-app setup command is gone from the mockup. Setting up an app is a full-window screen (page
   `setup`, no sidebar) with sections, a live deploy.yaml preview and drafts (`D.drafts`), item `p2b5`.
   Later the same day the owner reshaped it: user code is a **project**; **apps** are only ready-made ones
   (renamed in the mockup, the tracker and the spec). Adding a project is a Vercel-like flow (Source → Analyze
   → Configure → Deploy → live): drop or choose a folder (it stays in place), or import from Git; the engine
   detects the stack (`p1c8`); all `.env` values are hidden by default; config goes to the project's
   `scripts/deploy/<name>/deploy.yaml` (the existing deploy root; the owner picks `<name>`, shipdeck never
   invents one; the field starts with the project's name) so GUI, TUI and CI share it.
   Git: `p2e1` import, `p2e2` deploy on push/release built on CI, `p2e3` built on the server (deferred).
   Open: the spec still uses "app" for the clients (TUI, CLI, desktop); ask if they become "clients".
   The title bar has a native menu bar (File, View, Server, Help), asked for by the owner; it reuses the
   right-click menu code (`menuItems`, `openMenu`). The owner moved the rest up there too: the title bar reads
   logo, back, forward, home | menus … title … theme, window buttons. The sidebar starts with the server switch.
2. `p1c6` (new, planned): back up shipdeck's settings and config to one file, export and import. The owner
   asked for it on 2026-10-31. Open: lock the file with a password? Include each app's `deploy.yaml`? The
   mockup's Settings window shows Export and Import.
3. The owner looks at the spec (`p1a1`).
4. `p1a2`: the move to `apps/` + `crates/`, then `p1b1`, the protocol crate.

## Gotchas

- Canvas pages: give the viewport `overflow: clip`. With only `overflow: hidden`, a `#id` in the URL scrolls the
  viewport and the pan/zoom maths goes wrong. Wait for the web font (`document.fonts.load`) before the first fit,
  and fit again on `ResizeObserver` until the reader moves.
- Screenshots of a page: `msedge --headless=new --disable-gpu --hide-scrollbars --window-size=W,H
  --virtual-time-budget=5000 --screenshot=<file> file:///<path>#<id>`; add `--force-dark-mode` for dark.
