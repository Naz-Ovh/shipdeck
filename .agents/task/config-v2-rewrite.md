# Task: build config format 2

Created: 2026-11-10. For the coding agent that builds it. A reviewer checks the
whole result at the end against this file and the design page.

## 1. Your job

Replace shipdeck's config, release layout and deploy pipeline with the design
in [docs/feature/config-v2.md](../../docs/feature/config-v2.md). Build all of
it, except the parts section 4 marks as "parse only". Work until every item
in section 8 is done and every check in section 9 passes. Then write the
report in section 10.

Shipdeck is in alpha. This is a clean break: the old config format is
removed, not kept beside the new one.

## 2. Read first, in this order

1. [AGENTS.md](../../AGENTS.md). Its rules are mandatory. Section 5 repeats
   the ones you are most likely to break.
2. [.agents/README.md](../README.md), then [.agents/handoff.md](../handoff.md).
3. [docs/feature/config-v2.md](../../docs/feature/config-v2.md), the design.
   Read all of it.
4. The example configs:
   - [reference.yaml](../../docs/feature/config-v2/reference.yaml): every key.
   - [standalone.yaml](../../docs/feature/config-v2/standalone.yaml): one app.
   - [universabile/](../../docs/feature/config-v2/universabile/): a real
     monorepo, as a project file plus one deploy file.
5. [docs/code/architecture.md](../../docs/code/architecture.md),
   [config.md](../../docs/code/config.md),
   [releases.md](../../docs/code/releases.md),
   [remote.md](../../docs/code/remote.md),
   [agent.md](../../docs/code/agent.md),
   [monorepo-ssh-tests.md](../../docs/code/monorepo-ssh-tests.md).
6. The "App-facing nginx settings" section of
   [nginx-config-management.md](../../docs/feature/nginx-config-management.md).

## 3. Source of truth

- **The design page decides.** When today's code and the design page
  disagree, follow the design page.
- **Do not guess.** When the design page is unclear, contradicts itself, or
  says nothing about a case you must handle, stop that part. Write the
  question in the report (section 10), and continue with other parts.
- **Do not change the design page or the example configs** to fit your
  code. If you find an error in them, write it in the report.

## 4. Scope decided by the owner

| Topic | Decision |
| --- | --- |
| Old configs | Removed. A file in the old format fails with unknown-key problems. No migration command. |
| Old releases | A rollback to a release without `.shipdeck/settings.json` is refused: "this release was made before config format 2; deploy again". |
| `nginx:` and TLS | **Parse only.** Read and check every rule in the design. A deploy whose config has `nginx:` stops at preflight: "nginx management is not built yet". No nginx file is written; no certificate is requested. |
| `oxmgr:` | **Parse only.** Read and check it. A deploy that uses it stops at preflight: "oxmgr is not built yet". |
| `systemd.instances`, `{{INSTANCE}}`, `@` in a process name | Reserved errors, as the design's "Planned: systemd instances" table says. |
| Project file name | `shipdeck.yaml` or `.shipdeck`, exactly. Both in one folder is an error. `shipdeck.yml` is not read. |
| App order | Apps run in name order. |
| Process name | Default `<name>-<app>`. `process.name` overrides it. |
| Static apps | Refuse exactly the keys the design's "A static site" section lists. Allow everything else. |

Open questions 1–8 at the end of the design page stay open. The table above
gives the interim behavior for the ones that affect this build. Do not decide
any others.

## 5. Rules you must not break

These come from AGENTS.md and from the owner's standing feedback.

- **No new dependency** without the owner's approval: no crate, no npm
  package, no server program. If one seems needed, stop that part and write
  why in the report. Note: `serde_json` sorts map keys; do not add
  `preserve_order` or `indexmap` for app order, because apps run in name
  order.
- **No hacks.** No output scraping, no hand-kept lists that guess another
  tool's behavior, no silent fix-up of bad input. Bad input is a clear error.
- **Rust rules:** no production `.unwrap()`, `.expect()` or unchecked
  indexing. No `let _ =`, `.ok()` or `.unwrap_or_default()` that drops an
  error in download, range or filesystem code. Checked arithmetic and
  `TryFrom` for untrusted sizes. No `unsafe`. No `.clone()`, `Arc` or `Mutex`
  added only to satisfy the borrow checker.
- **Shell safety:** every value put into a server script goes through the
  existing `sh_quote`. Never join unquoted text into a script. Runner code
  goes to the runner as one argument.
- **File size:** aim for 300–550 lines. Hard caps: 800 lines per file, 100
  lines per function. `lib.rs` and `mod.rs` hold declarations and re-exports
  only.
- **Tests** go in a sibling file. New test files are named `<module>_tests.rs`.
  Do not rename existing test files.
- **Comments:** one line per function, method, type or field; at most five
  lines for a complex one. Longer explanations go in `docs/code/*.md`.
- **Types first.** For every non-trivial module, write the public types and
  signatures before the bodies.
- **Tests:** do not weaken an assertion or delete a test that still covers
  current behavior. Tests of the removed old format are rewritten for the new
  format, or deleted when the behavior no longer exists. List each deleted
  test in the report with the reason.
- **The TUI never does server setup.** Server setup is
  `scripts/server-setup.sh` only. Do not add a setup action, key or screen.
- **Do not refactor unrelated code.** If you find a bug or security problem
  outside this task, write it in the report; do not fix it silently.
- **Git:** do not commit. Leave every change in the working tree for review.

## 6. Target model

Write these types first, in `crates/engine/src/config/`. Names are a
suggestion; the shapes are not.

- **Project file:** `deploys: BTreeMap<String, String>` (deploy name → path
  of its deploy file).
- **Deploy file:** `name`, `server { ssh, dir }`, `release`, `apps`.
- **Release:** `files`, `exclude`, `shared`, `steps`.
- **App:** `source`, `files`, `exclude`, `binary`, `shared`, exactly one of
  `process` or `static: true`, an optional `nginx`, `steps`.
- **Process:** `name`, `start`, `env`, `env_file`, and exactly one manager:
  `Systemd { user, options | template }`, `Pm2 { options | ecosystem }`,
  `Oxmgr { options }`. Model the manager as an enum, so two managers cannot
  be represented.
- **File entry:** a path, or `{ path, to }`. `binary` uses the same type.
- **Steps:** the 12 steps of the design. Each has `local` and `server`
  command lists, plus its own settings from the design's "Settings" table.
- **Command:** `Sh(String)` for a plain string, or `{ <runner>: <code> }`
  with `Runner` an enum of `sh`, `bash`, `zsh`, `node`, `python3`. A map with
  any other key, or with more than one key, is a problem that suggests
  quotes, because an unquoted `a: b` in YAML is read as a map.
- **Placeholders:** one type that knows every name in the design's
  "Placeholders" table, where each is valid, and its value.

Then a resolved form used by everything after loading: one `Deployment` with
a list of apps. **There is one deploy pipeline**, not a single-app one and a
monorepo one. A single app is a deployment with one app. Merge
`deploy/mod.rs`, `deploy/stages.rs` and the `deploy/multiple*.rs` files into
that one pipeline, split by step and responsibility.

`.shipdeck/settings.json` stores the resolved deploy **with placeholders
unresolved**. A rollback resolves them again with the restored release's own
values (its `{{RELEASE}}`, `{{RELEASE_ID}}`, `{{VERSION}}`).

## 7. Where the work is

These paths hold today's behavior. Read each before changing it.

| Area | Paths |
| --- | --- |
| Config types, YAML, checks, schema | `crates/engine/src/config/` (all files), `docs/schema/deploy.schema.json` |
| Discovery (to remove) | `config/discover.rs`, `config/deployment.rs`, `apps/cli/src/main.rs`, `crates/engine/src/rpc/session.rs`, `crates/engine/src/rpc/workspace.rs`, `crates/tui/src/app/mod.rs`, `crates/engine/src/init.rs`, `apps/cli/src/init.rs` |
| Files and packing | `crates/engine/src/bundle/` (all files) |
| Release names, `release.json` | `crates/engine/src/release.rs` |
| Pipeline and steps | `crates/engine/src/deploy/` (all files) |
| Local commands | `crates/engine/src/build/mod.rs` (`run_local`) |
| Server commands, layout, backups | `crates/engine/src/remote/` (all files) |
| Units and PM2 files | `crates/engine/src/bundle/unit.rs`, `bundle/pm2.rs`, `bundle/default.service`, `remote/pm2.rs`, `remote/control.rs` |
| Rollback | `deploy/recover.rs`, `deploy/rollback.rs`, `deploy/multiple_recover.rs`, `deploy/backup.rs` |
| Agent and protocol | `crates/agent/src/releases.rs`, `crates/agent/src/inspect_release.rs`, `crates/protocol/src/message/common.rs`, `crates/protocol/src/message/agent.rs` |
| RPC and CLI flags | `crates/engine/src/rpc/convert.rs`, `rpc/operations.rs`, `apps/cli/src/main.rs` (`--on-failure`) |
| TUI | `crates/tui/src/` (`deployables.rs`, `modal.rs`, `releases.rs`, `release_details.rs`, `app/`) |
| Server setup | `scripts/server-setup.sh` (the sudo rule names `current/$SERVICE.service`) |
| Manual | `apps/cli/manual/index.yaml`, `configs.yaml`, `details.yaml` |
| SSH tests | `tests/monorepo_ssh.py` |

## 8. Work, in order

Finish each part, with its tests passing, before the next.

### Part 1. Config

1. The types of section 6, with the YAML reader keeping line and column for
   every problem, as today.
2. The path rules of "Paths on your PC": `/x`, `apps/x` (from the folder
   shipdeck runs in), `./x` and `../x` (from the config file's folder).
3. Every check the design names: required and refused keys per app kind,
   one manager, reserved systemd and PM2 properties, `template` with
   `options`, `ecosystem` with `start`, `proxy_pass` versus `root` and
   `index`, reserved names, paths in the release, `to` required outside the
   project or source.
4. Placeholders: the name pattern, spaces inside, unknown names, names used
   where they have no value (for example `{{APP}}` in `release:`, release
   names in `preflight`). Every check runs when the file is read.
5. `just schema` writes the new schema.

### Part 2. Project file and commands

1. Remove discovery. Shipdeck reads `shipdeck.yaml` or `.shipdeck` in the
   current folder, or the deploy file given by `--shipdeckfile=<path>`.
2. Commands take a deploy name instead of a deploy root:
   `shipdeck deploy <deploy>`, and the same for `rollback`, `releases`,
   `server-info`, `work` and the rest. Without a project file and without
   `--shipdeckfile`, stop and name the file expected.
3. `--on-failure` overrides `rollback.mode`. Read what each value does today
   in `rpc/convert.rs` and the CLI, then map it to `auto` or `off` without
   changing its meaning. Write the mapping in the report.
4. The TUI lists the deploys of the project file and runs them. Every
   deploy has a list of apps; keep the existing screens working for it. Add
   no new screens beyond what that needs.
5. `shipdeck init` writes a project file and a deploy file in the new format.

### Part 3. Layout, packing and `shipdeck tree`

1. Files land exactly as "Where files land" says: `release.files` at the
   release root by project path, app files under `apps/<app>/` by path inside
   `source`, `to` when given, `binary` by file name or its `to`, with mode
   `0755`.
2. Shipdeck's own files go in `.shipdeck/`: `release.json`, `settings.json`,
   `<process>.service`, `<process>.pm2.json`.
3. Shared folders: the link in the release, the data at the same path under
   `shared/`.
4. Every error in "Errors instead of guesses".
5. `shipdeck tree` with `--app`, `--depth`, `--build` and `--json`, as the
   design's `shipdeck tree` section says. It never connects to the server and
   uses the same file-collecting code as `deploy`.
6. The agent and the protocol read `release.json` and the process files from
   `.shipdeck/`.
7. `scripts/server-setup.sh` installs units from
   `current/.shipdeck/$SERVICE.service`.

### Part 4. Steps and runners

1. The 12 steps, each as "your commands, then shipdeck's action", in the
   order and with the actions of "The 12 steps, in order".
2. Every step takes `local` and `server`, at release and app level, run in
   the folders of "Where your commands run". `server` runs before `local`;
   release commands before app commands; apps in name order; `health` checks
   apps before the release.
3. `build` names the release before its commands run.
4. `health`: each try runs the commands, then fetches `health.url`, until
   `timeout_secs`.
5. `failed.server` runs only when the server can be reached; otherwise log
   that it was skipped.
6. Runners: local commands start the runner directly with its flag and the
   code as one argument (`sh -c`, `bash -c`, `zsh -c`, `node -e`,
   `python3 -c`). Server commands do the same inside the server script,
   quoted with `sh_quote`. Keep the existing Windows `sh` handling for local
   `sh` commands.
7. Preflight collects the runner names from the config, without parsing
   command text, and checks them: local runners on this PC, server runners on
   the server, together with `preflight.requires`. A missing runner names the
   runner, the step and the app.
8. Environment variables: `SHIPDECK_<NAME>` for every placeholder, plus
   `SHIPDECK_ACTION`, `SHIPDECK_STEP`, `SHIPDECK_PREVIOUS`, `SHIPDECK_ERROR`.
   Remove the old names the design's change table lists.

### Part 5. Backups and rollback

1. A backup holds the uploaded files and `.shipdeck/`, plus `include`, minus
   `exclude`, per release and per app. Shared folders never.
2. `backup.keep` with `open` and `compressed`, as today.
3. `rollback` with `mode` (`ask`, `auto`, `off`), `ask_secs` and
   `keep_failed`. This replaces the `ROLLBACK_ANSWER_SECS` constant.
4. The rollback sequence and the "Which commands run in a rollback" table,
   exactly. `setup.server` runs again only when the release comes back from
   a backup.
5. Where rollback commands come from: `setup.server`, `health.server` and
   `done` from the restored release's `settings.json`; `rollback` and
   `failed` from the deploy file shipdeck was started with.
6. A rollback checks the restored release with its own health checks.
7. A rollback by hand runs `failed` only if the rollback itself fails.
8. Removing the failed release (when `keep_failed` is false) reports its
   error. Today's `let _ = server.discard(...)` in `deploy/recover.rs` drops
   it; the new code must not.

### Part 6. Process managers

1. systemd: `user`; `options` by section (`Unit`, `Service`, `Install`),
   replacing a built-in line of the same name or adding it, a list writing
   one line per item; `template` as a path on your PC; the reserved
   properties refused.
2. The built-in unit and custom templates use the new placeholder names.
   Today's `{{dir}}`, `{{workdir}}` and the rest of `unit::PLACEHOLDERS` are
   replaced.
3. PM2: `options` with the reserved keys refused (today they silently
   replace shipdeck's `name` and `cwd`); `ecosystem` as a path in the app
   folder.
4. oxmgr and nginx: parse only, as section 4 says.

### Part 7. Docs, examples and records

1. Rewrite every config in `examples/` in the new format, with a project
   file each. Comments follow the example rule: one plain sentence saying
   what a thing is, plus a docs link.
2. Update the manual in `apps/cli/manual/`.
3. Update the website pages under `docs/website/content/docs/` that show
   config, layout, steps, variables or commands. Remove
   `guides/deploy-root.mdx` and its links. Add an "Upgrading" entry that says
   the old format is gone and points at the design.
4. Update `docs/code/config.md`, `releases.md`, `remote.md`, `agent.md`,
   `architecture.md`, `development.md` and `monorepo-ssh-tests.md`.
5. Update `docs/visual/Plan/spec.html` where it shows the config, the server
   folders or the steps.
6. In `docs/project-progress.html`, add a row for this work with status
   `built`, and update the "Up next" box and `UPDATED`.
7. Add today's section to `.agents/handoff.md`.

## 9. Checks

All must pass. Never claim a check you did not run.

1. `cargo fmt --all -- --check`
2. `cargo check --workspace --all-targets`
3. `cargo test --workspace`
4. `just lint`: strict Clippy with pedantic and perf warnings as errors.
5. `just schema`, then `git diff --exit-code docs/schema` shows no change.
6. `just verify`.
7. `just build-doc`: the website builds.
8. `shipdeck tree` on a copy of `docs/feature/config-v2/universabile/` with
   placeholder build output. The tree matches the design's example, apart
   from file counts.
9. The Podman SSH suite in `docs/code/monorepo-ssh-tests.md`, with
   `tests/monorepo_ssh.py` rewritten for the new format. It must cover:
   single app, several apps, release files, `to`, `.shipdeck/`, shared
   links, each runner, a failure before the switch, an automatic rollback, a
   rollback by hand to an open release, a rollback from a backup (with
   `setup.server` run again), `keep_failed`, and the preflight runner check.
   If you cannot run it, say so in the report; do not mark it passed.

## 10. The report

Write `.agents/task/config-v2-rewrite-report.md` with:

1. **Summary:** what was built, in five lines at most.
2. **Per part (1–7):** what changed, and the main files.
3. **Decisions:** every choice the design did not make, and why.
4. **Questions:** every place you stopped because the design was unclear.
5. **Deleted tests:** each one, with the reason.
6. **Checks:** each check of section 9, with the command and its result. Mark
   unrun checks as unrun.
7. **Problems found outside the task**, not fixed.
8. **Known gaps:** anything in the design not built, other than the
   parse-only parts of section 4.
