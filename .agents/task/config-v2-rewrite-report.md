# Config format 2: what was built

Task: `.agents/task/config-v2-rewrite.md`, plus the `binary` steer in
`.agents/task/config-v2-steer-01-binary.md`, which wins over the earlier `binary`
design. Nothing is committed; the work sits in the working tree for review.

## The shape of the change

The config is rebuilt as one clean break, not a compatibility layer.

**Two files.** The project file (`shipdeck.yaml` or `.shipdeck`, exactly those two
names) holds only `deploys:`: a name and the deploy file of each. A deploy file
holds `name`, `server`, `release` and `apps`. It lives anywhere, under any name,
and needs no project file with `--shipdeckfile`.

**One `Deployment`.** `crates/engine/src/config/resolve.rs` resolves a deploy file
into one `Deployment`, and only that reaches the pipeline. Before this there were
three shapes (single service, static site, monorepo) with their own paths through
the code; now there is one.

**Apps inline.** `apps.<app>` is written in the deploy file. There is no separate
app file and no copying of `release` settings into apps.

**The manager is the block.** `process.systemd`, `process.pm2` or
`process.oxmgr`. There is no `manager:` key and no default: a process with no
block is an error, and one with two is an error.

**`binary` per the steer.** `apps.<app>.binary` takes a path, or
`{ path, to }`. It lands in the app folder with mode 0755, and the default
`process.start` follows it (`./api`, or `./bin/api` with `to`). A folder or a
pattern is refused, naming the path, and `to` may not be `.shipdeck`, climb out
with `..`, or start with `/`.

**Twelve steps, one pipeline.** preflight, build, upload, setup, backup, switch,
restart, nginx, health, done, rollback, failed. `pack` and `unpack` are gone:
build collects, upload packs, sends, unpacks and links. Every step takes `local`
and `server`, and every runner used is checked in preflight.

**Runners.** `sh` (the default), `bash`, `zsh`, `node`, `python3`. A command is
text or one runner and its code.

**Placeholders.** One table in `config/placeholders.rs`, resolved in paths,
commands, `env`, nginx and unit templates, and also exported as
`SHIPDECK_<NAME>`. A name used where it has no value is an error that says so.

**Shipdeck's own files in `.shipdeck/`.** `release.json`, `settings.json`, the
unit or PM2 file, `oxfile.toml` and nginx configs. The rest of a release is only
the user's files, so `shipdeck tree` reads as a list of them.

**A rollback restores the older release's settings.** `.shipdeck/settings.json`
holds the resolved deploy, and a rollback reads its `setup.server`, `health` and
`done`. A release from before format 2 has none and is refused with a message
that says to deploy again.

**Parsed, not built.** `nginx:` and `oxmgr:` are read and checked; a deploy that
uses either stops at preflight. `systemd.instances`, `{{INSTANCE}}` and `@` in a
process name are refused with the same reason.

**New: `shipdeck tree`.** Every path the next release will hold, from local files
only, sharing the release plan with `deploy`, so the two cannot disagree.
`--depth`, `--app`, `--build`, `--json`.

## Where it lives

- `crates/engine/src/config/` — `types.rs`, `steps.rs`, `commands.rs`,
  `values.rs`, `placeholders.rs`, `paths.rs`, `names.rs`, `check.rs`,
  `rules.rs` (with `rules/app_rules.rs` and `rules/file_rules.rs`), `problem.rs`,
  `load.rs`, `resolve.rs`, `schema.rs`, `yaml.rs`, `glob.rs`.
- `crates/engine/src/bundle/plan.rs` — the release plan. `content.rs` hashes,
  fingerprints and packs it.
- `crates/engine/src/deploy/` — one pipeline, split by step; `recover.rs` is the
  rollback after a failed switch, restart, nginx or health.
- `crates/engine/src/tree.rs`, `crates/engine/src/init.rs`.
- `crates/engine/src/ssh/` — `pool.rs` (sessions) and `splitter.rs` (lines) were
  split out of `mod.rs`, which now holds `Session` only.
- `apps/cli/src/naming.rs` — deploy naming and the two setup answers, split out
  of `main.rs`.
- `crates/tui/src/deployables.rs` — the apps panel is its own function.

Every Rust file is under the 800-line cap; `rules.rs` was 1163 and is now three
files.

## Checks

Run here, in this order:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo check --workspace --all-targets` | pass, no warnings |
| `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::perf` | pass |
| `cargo test --workspace` | 361 passed, 0 failed |
| `just schema` twice, `md5sum -c` | stable; `deploy.schema.json` and `protocol.schema.json` unchanged on the second run |
| `just verify` | pass |
| `just build-doc` | pass, after fixing MDX tag errors in the new pages |

`shipdeck tree` on the design's `universabile` example matches the layout in
`docs/feature/config-v2.md`.

### Not run

`tests/monorepo_ssh.py` is rewritten for format 2 with the scenarios the task
lists, but was not run: this machine has no Podman container named
`shipdeck-monorepo-ssh-test` and no `sshd` (`which sshd` finds nothing). It is
written to fail with the missing name rather than skip a scenario. Check 9 in
the task is therefore **unrun**, not passed.

`python3 -m py_compile tests/monorepo_ssh.py` passes, so the file is at least
valid Python.

## Tests added

- `config/schema_tests.rs`: `every_example_parses_and_resolves` reads each folder
  in `examples/`, its project file and every deploy file it names, and parses
  them. This is the test the examples were missing: before it, nothing read them.
- `config/rules_tests.rs` and the rest of the config suite cover the new rules:
  refused keys per kind, one manager, `binary` forms and refusals, placeholder
  scope, release paths, shared overlap.

## Docs

Rewritten for format 2:

- `apps/cli/manual/index.yaml` and `details.yaml`: every page, including the new
  `monorepo` framing and `step.rollback`.
- `README.md`.
- `docs/code/config.md`, `docs/code/architecture.md`, `docs/code/releases.md`,
  `docs/code/remote.md`, `docs/code/development.md`,
  `docs/code/monorepo-ssh-tests.md`.
- `docs/website/content/docs/`: `index`, `troubleshooting`, `monorepos`,
  `concepts/how-it-works`, `concepts/security`;
  `getting-started/{quickstart,installation}`;
  `guides/{pipeline,environment,ssh,server-access,health-and-rollback,releases,
  examples,upgrading}` and a new `guides/files-and-layout`;
  `reference/{config,cli,screens,server-layout,config-problems,variables}`.
  `reference/config.mdx` replaces `shipdeck-yaml.mdx`; `guides/deploy-root.mdx`,
  `guides/packing-files.mdx`, `reference/service-template.mdx` are gone, with
  their content folded into `config`, `files-and-layout` and `pipeline`.
- `docs/website/lib/pipeline.ts` (twelve steps), `app/(home)/page.tsx`,
  `lib/llms.ts`, and the two `meta.json` files.
- `docs/visual/Plan/spec.html` and `docs/project-progress.html` (row `p0c7`,
  "Up next", `UPDATED`), plus `.agents/handoff.md`.

## What is deliberately not there

- No migration command and no compatibility mode. An old file fails with
  unknown-key problems, and `guides/upgrading.mdx` maps every old key to its new
  place.
- No `nginx` or `oxmgr` runtime. Both are parsed and checked only.
- No `systemd.instances`.
- Nothing committed.

## Open items for the owner

1. Run `tests/monorepo_ssh.py` against a container with systemd, PM2 and sshd.
   It is the one task check that could not be exercised here.
2. Decide the release version. The config break and the `.shipdeck/` unit path
   are breaking changes, so a major release is needed.

## The first review, and what it changed

Review: `docs/review/config-v2-rewrite-review.md`. It found three P1s that break
the design for users. All three are fixed, and each has tests.

**P1: placeholders were never replaced in commands.** `placeholders::expand`
ran only for units, so `{{ROOT}}` and `{{VERSION}}` reached the shell as text.

- One shared `Deployment::values` and `Deployment::expand` hold the values;
  `Deployment::systemd` and `Deployment::pm2` give typed access to the manager.
- `Exec::line` expands a command before it runs, for both sides. Local
  commands pass the expanded line to `build::run_command`, which now takes it
  as its own argument.
- `Exec::env` expands each `process.env` value and the `env_file` name.
- The built-in unit's `Environment=` values and the PM2 file (`script`,
  `interpreter`, every string in `pm2.options`) expand too.
- `health.url` expands in both the deploy and the rollback health loop.
- Tests: a server command, a local command and an `env` value.

**P1: an automatic rollback skipped the rollback commands.** `restore_back`
now runs `rollback.server` and `rollback.local` from the running deploy file
before the action, with `SHIPDECK_PREVIOUS` naming the release being put back,
exactly as `step_put_back` does for a rollback by hand. `tests/monorepo_ssh.py`
now has the automatic-rollback scenario write a `rollback.local` and a
`rollback.server` line and assert both ran.

**P1: `shipdeck deploy` always overrode `rollback.mode`.** `--on-failure` is now
`Option<OnFailureArg>`, with no default. The CLI sends it only when given, so
`mode: off` keeps the failed release. `Console.Choices.on_failure` is
`Option<OnFailure>`; an open rollback question with no flag is answered
"roll back", as before. Tests: no flag leaves the file's mode alone, a flag
still wins, and the request carries no override.

Two P2s were already fixed before this review and are confirmed here:
`just lint` passes (the two long functions are split), and no file is over 800
lines (`config/rules.rs` is three files; `config/types.rs` is split).

The other P2s:

- **Dead code removed.** Six `#[allow(dead_code)]` functions are gone
  (`activate.rs`, `build.rs`, `preflight.rs`, `recover.rs`), and the `let _ =`
  silencers with them: `pipeline` drops the unused name, `check_binaries` no
  longer names an unused key, `step_put_back` and `rollback_preflight` no longer
  carry a `Server` nothing uses, and `tools` no longer names an unused scope.
  `plan.rs` stopped copying a path out and back in.
- **The manager is an enum.** `Process.manager` is `Option<KeepUp>`, so no value
  holds two managers. A `RawProcess` gives the same schema and the same YAML
  shape; its `Deserialize` refuses two keys, and a test reads two keys straight
  into `Process` and expects an error.
- **`binary: { path }` without `to`.** `MappedFile.to` is `Option`, so the map
  form without `to` behaves like the plain form. `binary_name` and
  `binary_place` fall back to the file name. A test covers it. The only schema
  change in this round is `to` becoming optional.

The P3s:

- **`deploy_completed` was recorded twice.** Removed from the done step; the
  pipeline records it once.
- **Placeholder nginx and oxmgr files in the release.** The plan no longer adds
  them, so `shipdeck tree` lists only files that will exist.

The records the review called missing are in the tree: the review was written
before the documentation pass. `docs/code/`, `docs/visual/Plan/spec.html`,
`docs/project-progress.html`, `.agents/handoff.md` and `guides/upgrading.mdx`
are all changed, and this report exists.

## Checks after the review

| Check | Result |
| --- | --- |
| `just fmt-check` | pass |
| `just lint` | pass, strict pedantic and performance |
| `cargo test --workspace` | 367 passed, 0 failed |
| `just verify` | pass |
| `just schema`, twice | stable; only `to` changed shape |
| `just build-doc` | pass |
| `tests/monorepo_ssh.py` | written, compiles, **not run**: no container, no `sshd` here |
