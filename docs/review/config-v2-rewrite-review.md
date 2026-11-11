# Config format 2 rewrite review

Reviewed on 2026-11-11.

**Scope:** the uncommitted rewrite of shipdeck's config, release layout and
deploy pipeline. It was checked against:

- the task, [config-v2-rewrite.md](../../.agents/task/config-v2-rewrite.md);
- the binary steer, [config-v2-steer-01-binary.md](../../.agents/task/config-v2-steer-01-binary.md);
- the design, [config-v2.md](../feature/config-v2.md).

## Result

The structure is right:

- one pipeline for one app and many apps;
- the 12 steps;
- `.shipdeck/` for shipdeck's own files;
- the project file and `--shipdeckfile`;
- the `binary` steer, in its main form;
- the nginx and oxmgr stops at preflight.

`shipdeck tree` prints a correct layout for the universabile deploy file.

The work is not ready:

- Three findings break the design for users (P1).
- Lint fails.
- Two files are over the size cap.
- Most of the records in task part 7 are missing.
- The report the task asks for was not written.

## Checks run

| Check | Result |
| --- | --- |
| `just fmt-check` | Pass. |
| `just test` | Pass. 225 engine tests, plus the other crates. |
| `just lint` | **Fail.** Two functions are over 100 lines. |
| `--schema` against `docs/schema/deploy.schema.json` | Same. The website copy is the same too. |
| `shipdeck tree --depth 3` on universabile | Pass. 2498 files; every app is under `apps/<app>`. |

## Findings

### P1: placeholders are never replaced in commands

**Where:**

- [commands.rs:63](../../crates/engine/src/deploy/commands.rs#L63)
- [commands.rs:221](../../crates/engine/src/deploy/commands.rs#L221)
- [build/mod.rs:33](../../crates/engine/src/build/mod.rs#L33)

`placeholders::expand` is called only for systemd units, in `bundle/unit.rs`.

These go to the shell as written, so `{{ROOT}}` stays the literal text `{{ROOT}}`:

- server commands;
- local commands;
- `process.env` values;
- PM2 files.

**Examples that break:**

- `test -w {{ROOT}}` in reference.yaml;
- `node --env-file="{{CONFIG}}/production/.env"` in the universabile build;
- `{{VERSION}}` in `examples/node/deploy.yaml`.

The design says placeholders work in every string. No test runs a command
that holds a placeholder.

**Required fix:**

- Expand every command, and every `env` value, with the step's values before it runs.
- Add tests for a server command, a local command and an `env` value.

### P1: an automatic rollback skips the rollback commands

**Where:** [recover.rs:68-93](../../crates/engine/src/deploy/recover.rs#L68-L93).

An automatic rollback goes straight to the restore. Only a rollback by hand
runs `rollback.server` and `rollback.local`, in `step_put_back`.

The design says both kinds run these commands first, before the action.

**Required fix:**

- Run the rollback commands from the current deploy file, before the restore.
- `SHIPDECK_PREVIOUS` names the release being put back.
- Add a test for each mode.

### P1: `shipdeck deploy` always overrides `rollback.mode`

**Where:**

- [main.rs:114](../../apps/cli/src/main.rs#L114)
- [console/run.rs:71](../../apps/cli/src/console/run.rs#L71)
- [rpc/operations.rs:35](../../crates/engine/src/rpc/operations.rs#L35)

`--on-failure` has a default of `rollback`, and the CLI always sends it. So
`rollback.mode: off` in the deploy file is never used from the CLI. Every CI
deploy rolls back.

**Required fix:**

- Make the flag optional.
- Send it only when it is given.
- Add a test: `mode: off` with no flag keeps the failed release.

### P2: lint fails, and two functions are over the 100-line cap

**Where:**

- `Run::recover`, at [recover.rs:21](../../crates/engine/src/deploy/recover.rs#L21), is 117 lines.
- The function at [init.rs:112](../../crates/engine/src/init.rs#L112) is 101 lines.

### P2: two files are over the 800-line cap

**Where:**

- [config/rules.rs](../../crates/engine/src/config/rules.rs) has 1163 lines.
- [config/types.rs](../../crates/engine/src/config/types.rs) has 831 lines.

**Required fix:** split them by responsibility. Examples:

- the systemd and PM2 rules into their own module;
- the step types into their own module.

### P2: unused code is kept alive

**Unused functions behind `#[allow(dead_code)]`:**

- [activate.rs:255-270](../../crates/engine/src/deploy/activate.rs#L255-L270), three functions;
- [build.rs:204-215](../../crates/engine/src/deploy/build.rs#L204-L215), two;
- [recover.rs:573](../../crates/engine/src/deploy/recover.rs#L573), one;
- [preflight.rs:487](../../crates/engine/src/deploy/preflight.rs#L487), one.

**`let _ =` lines that only silence an unused variable:**

- [run.rs:215](../../crates/engine/src/deploy/run.rs#L215)
- [build.rs:118](../../crates/engine/src/deploy/build.rs#L118)
- [activate.rs:97](../../crates/engine/src/deploy/activate.rs#L97)
- [recover.rs:223](../../crates/engine/src/deploy/recover.rs#L223)
- [preflight.rs:193](../../crates/engine/src/deploy/preflight.rs#L193)
- [rules.rs:801](../../crates/engine/src/config/rules.rs#L801)

The project rules forbid broad `#[allow]` and unused code.

[plan.rs:443-444](../../crates/engine/src/bundle/plan.rs#L443-L444) is also
dead: it copies `entry.path` out and then back in.

**Required fix:**

- Delete the unused functions.
- Remove the unused parameters and variables, instead of silencing them.

### P2: the process manager is not an enum

**Where:** [types.rs:135-143](../../crates/engine/src/config/types.rs#L135-L143).

`Process` holds three `Option` fields: `systemd`, `pm2` and `oxmgr`. Task
section 6 asks for an enum, so two managers cannot be represented. The rules
catch two managers, but the type still allows them.

### P2: `binary: { path }` without `to` is refused

**Where:** [types.rs:322-327](../../crates/engine/src/config/types.rs#L322-L327).

`MappedFile.to` is required. The steer says the map form without `to`
behaves like the plain form. No test covers it.

### P3: `deploy_completed` is recorded twice

**Where:**

- [activate.rs:246](../../crates/engine/src/deploy/activate.rs#L246)
- [run.rs:223](../../crates/engine/src/deploy/run.rs#L223)

Every deploy writes two events on the server.

### P3: placeholder nginx and oxmgr files in the release

**Where:** [plan.rs:395-435](../../crates/engine/src/bundle/plan.rs#L395-L435).

The plan adds `.shipdeck/nginx/*.conf` and `.shipdeck/oxfile.toml` with the
text "not built yet". `shipdeck tree` then lists files that will never be
real.

A deploy with nginx or oxmgr stops at preflight anyway. So `tree` should not
list these files until those parts are built.

### P3: records and the report are missing

**Not done from task part 7:**

- `docs/code/` pages: none changed.
- `docs/visual/Plan/spec.html`.
- `docs/project-progress.html`.
- `.agents/handoff.md`.
- The "Upgrading" entry. `guides/upgrading.mdx` is unchanged.

**Report:** `.agents/task/config-v2-rewrite-report.md` does not exist. So
these are unknown:

- the agent's decisions;
- its open questions;
- the result of each check, by its own account.

**Deleted tests:** 110 tests were in deleted files; the new files have 85. With
no report, it is not possible to see which old tests were dropped, and why.

## Second review

Reviewed on 2026-11-11, after the agent said the work was finished. The
report, [config-v2-rewrite-report.md](../../.agents/task/config-v2-rewrite-report.md),
now exists.

### Checks run

| Check | Result |
| --- | --- |
| `just fmt-check` | Pass. |
| `just lint` | Pass. |
| `just test` | Pass. 361 tests. |
| `--schema` against `docs/schema/deploy.schema.json` | Same. |

### Status of each finding

| Finding | Status |
| --- | --- |
| P1: placeholders are not replaced in commands | **Open.** Reproduced; see below. |
| P1: an automatic rollback skips the rollback commands | **Open.** The code moved into `restore_back` ([recover.rs:101](../../crates/engine/src/deploy/recover.rs#L101)); it still runs no rollback commands. |
| P1: `shipdeck deploy` always overrides `rollback.mode` | **Open.** The default is still at [main.rs:116](../../apps/cli/src/main.rs#L116). |
| P2: lint fails | Fixed. |
| P2: files over 800 lines | Fixed. `rules.rs` is split into `rules/`; `types.rs` is split into `steps.rs`, `commands.rs` and `values.rs`. |
| P2: unused code kept alive | **Open.** The same 7 `#[allow(dead_code)]` functions and 6 `let _ =` lines. One `let _ =` moved to [file_rules.rs:36](../../crates/engine/src/config/rules/file_rules.rs#L36). |
| P2: the process manager is not an enum | **Open.** |
| P2: `binary: { path }` without `to` is refused | **Open.** `MappedFile.to` is still required, at [values.rs:104](../../crates/engine/src/config/values.rs#L104). |
| P3: `deploy_completed` is recorded twice | **Open.** |
| P3: placeholder nginx and oxmgr files | **Open.** |
| P3: records and the report | Mostly fixed. The docs, spec, tracker, handoff and upgrading page are updated. Gaps are listed below. |

### The placeholder bug, reproduced

A scratch project had this build command:

```yaml
release:
  steps:
    build:
      local: echo "version={{VERSION}} project={{PROJECT}}" > web/out.txt
```

`shipdeck tree prod --build` ran it. The file then held:

```
version={{VERSION}} project={{PROJECT}}
```

Deploys run local commands through the same function,
`build::run_command`. Server commands go through `server_line`, which also
returns the text unchanged.

### The report

**A wrong claim:** the report says placeholders are resolved in commands and
`env`. The code and the run above show they are not.

**Missing parts.** Task section 10 asks for 8 parts. These are not there:

- the decisions the agent made;
- its open questions;
- which old tests were deleted, and why. The deleted test files held 110
  tests.

## Limits

These were not run in this review:

- the Podman SSH suite, `tests/monorepo_ssh.py`;
- `just build-doc`.

Not read line by line:

- the TUI;
- the protocol;
- the website pages.

The findings above come from the engine, the CLI and the checks.
