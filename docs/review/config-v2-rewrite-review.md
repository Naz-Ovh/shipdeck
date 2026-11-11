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

## Third review

Reviewed on 2026-11-11 after steer 2,
[config-v2-steer-02-cli-and-fixes.md](../../.agents/task/config-v2-steer-02-cli-and-fixes.md).

**Scope:** the changes since checkpoint commit `57f11c7` on branch
`config-v2`: 57 files. No file changed between 09:28:26 and the end of this
review.

### Checks run

| Check | Result |
| --- | --- |
| `just fmt-check` | **Fail.** `apps/cli/src/` (`main.rs`, `init.rs`, `naming.rs`, `main_tests.rs`, `target_tests.rs`) is not formatted. |
| `just lint` | **Fail.** 7 errors; see below. |
| `just test` | **Fail.** 344 pass. One fails: `the_published_schema_is_current` in `crates/protocol`. |
| `--schema` against `docs/schema/deploy.schema.json` | Same. |

### Part B: status of each finding

| Finding | Status | Evidence |
| --- | --- | --- |
| B1: placeholders in commands | **Fixed.** | The reproduction now writes `version=1.2.3` and the real project path. Test: `a_local_command_writes_the_expanded_value`. |
| B2: an automatic rollback skips the rollback commands | **Fixed.** | `restore_back` runs them first. Tests: `an_automatic_rollback_runs_with_mode_auto`, `an_ask_answered_yes_rolls_back`, `the_rollback_commands_see_the_release_being_put_back`. |
| B3: `--on-failure` overrides `rollback.mode` | **Fixed.** | The flag is an `Option`. Test: `mode_off_keeps_the_failed_release`. |
| B4: unused code | **Fixed.** | The steer's grep finds nothing. |
| B5: the manager as one choice | **Fixed.** | `resolve::Manager` carries its block. Test: `the_manager_carries_its_own_block`. |
| B6: `{ path }` without `to` | **Fixed.** | Test: `a_binary_and_a_file_take_a_map_with_no_to`. |
| B7: `deploy_completed` recorded twice | **Fixed.** | One call is left, in `run.rs`. |
| B8: placeholder nginx and oxmgr files | **Fixed.** | They are no longer in the plan. |
| B9: full paths in file errors | **Fixed.** | Test: `a_missing_binary_names_its_full_path`. |

### Part A: the command line, run by hand

| Case | Result |
| --- | --- |
| `tree prod` (a name) and `tree deploy.yaml` (a file) | Both work. |
| `tree ./prod` and `tree prod.v2` | Refused, with the ending and name rule. |
| `-C <dir> -c shipdeck.yaml` run from another folder | Works; `-c` starts from `-C`. |
| `-c` naming a deploy file | Refused: "is a deploy file, not a root file", with the right help. |
| A root file given as the deploy argument | Refused: "is a root file, not a deploy file". |
| An unknown name | Names the root file and the names it lists. |
| No root file in the folder | Names the folder and the two names it expected. |
| A deploy written in the root file, through `-c ops/root.yaml` | `./note.txt` and `{{CONFIG}}` start from `ops/`. A problem points at its line in the root file. |

### Open findings

#### P2: the checks fail

**Lint errors:**

- `config/load.rs:342`: "case-sensitive file extension comparison", twice. The
  code lowercases first, so the lint is a false alarm here. Rewrite the check
  with `Path::extension` and `eq_ignore_ascii_case`, which also satisfies the
  lint. Don't silence it with `#[allow]`.
- `remote/control.rs:78`: an argument passed by value but not consumed.
- `deploy/run_tests.rs:277, 286, 313, 348`: a `BTreeMap::default()` style
  error, and three `mut` variables that don't need it.

**The failing test:** `docs/schema/protocol.schema.json` is out of date, so
`just protocol` was not run after the protocol messages changed.

**Formatting:** `cargo fmt` was not run on `apps/cli/src/`.

#### P2: `--help` still names the old flag and the old file

**Where:** [main.rs:48-53](../../apps/cli/src/main.rs#L48-L53) and
[main.rs:103](../../apps/cli/src/main.rs#L103).

`shipdeck --help` still tells users about `--shipdeckfile` and "a project
file".

#### P3: two error headings name the wrong file

When `-c` names a file without `root: true`, or names a deploy file, the
heading says "1 shipdeck.yaml file has 1 problem". The file is
`noroot.yaml` or `deploy.yaml`.

The help for a missing `root` says "add `root:`". It should say
"add `root: true`".

#### P3: docs, examples and records were not updated (task A6 and "When it is done")

**The old terms remain.** `--shipdeckfile` or "project file" is still in:

- the CLI manual: `apps/cli/manual/details.yaml` (25 times) and `index.yaml`;
- website pages: `reference/cli.mdx` (16), `troubleshooting.mdx` (7),
  `reference/config.mdx`, `config-problems.mdx`, `index.mdx`, `upgrading.mdx`,
  `quickstart.mdx` and `installation.mdx`;
- `docs/code/config.md`, `README.md`, `docs/project-progress.html`;
- the header comment of every `examples/*/shipdeck.yaml`.

**Missing example:** no example has a deploy written in the root file.

**Report and handoff:** neither has changed since the checkpoint. The report
has no table for steer 2 and no list of deleted tests.

## Fourth review

Reviewed on 2026-11-11, after the agent said the third-review findings were
fixed. No file changed after 09:56:11.

### Checks run

| Check | Result |
| --- | --- |
| `just fmt-check` | Pass. |
| `just lint` | Pass. |
| `just test` | Pass. 398 tests. |
| `--schema` against `docs/schema/deploy.schema.json` | Same. The website copy is the same too. |

### Status of the third-review findings

| Finding | Status | Evidence |
| --- | --- | --- |
| P2: the checks fail | **Fixed.** | All four checks above pass. |
| P2: `--help` names the old flag and file | **Fixed.** | `shipdeck --help` shows `-C`, `-c` and "root file". |
| P3: error headings name the wrong file | **Fixed.** | Run by hand: "1 problem in noroot.yaml", and "add `root: true`". |
| P3: docs, examples and records | **Mostly fixed.** | The old terms are gone outside `.agents/` and `docs/review/`. `examples/inline/` is added. The report and handoff are updated. Two gaps are listed below. |

The agent's one deviation is accepted. `Manager` owns its block instead of
borrowing it, because `Server` moves into `tokio::spawn`, which needs
`'static`. The intent of B5 holds: two managers cannot be represented.

### Open findings

#### P3: the agent edited the design page

The task says the design page decides, and the agent does not edit it. It
changed three places in [config-v2.md](../feature/config-v2.md):

1. The `oxfile.toml` and `nginx/*.conf` rows now say "Not written today".
2. The nginx files are removed from the `shipdeck tree` example.
3. The "What changes" row for `--shipdeckfile` no longer names the flag. This
   was only to pass the steer's grep.

Edits 1 and 2 mix today's build state into the target design. The design
should keep showing where nginx files will land. The owner decides whether to
keep or revert them.

#### P3: deleted tests from the first round are not listed

The report's "Deleted tests" table covers only this round's renames: 10
tests. The first round deleted test files that held 110 tests. Those are not
mapped to new tests, and no reason is given for dropping them. Most covered
the old format, but the report should say so for each file.

#### P3: the inline example's header explains internals

[examples/inline/shipdeck.yaml](../../examples/inline/shipdeck.yaml) opens
with six comment lines about checks, line numbers and path rules. The
example rule is one plain sentence saying what the file is, plus a docs link.

## End-to-end test on a real server

Run on 2026-11-11 against an instabox (Debian 13, systemd, Node 22, PM2 7.1.2).
The real CLI deployed a scratch project over SSH with three apps: a systemd
API with a binary and a shared folder, a PM2 worker, and a static site.

**Setup:**

- The test logged in as `user` through an SSH tunnel to the box's own sshd.
  The gateway's token user name is not an account on the box.
- The agent was a local musl build, through `SHIPDECK_AGENT_BINARY`.

Everything was removed afterwards.

### Passed

| Scenario |
| --- |
| First deploy: release files, `to`, `.shipdeck/`, the binary at mode 755, the shared link. |
| Placeholders expand in server and local commands. |
| The systemd API and the PM2 worker run, and `env_file` is loaded. |
| A second deploy keeps shared data. |
| Retention: `keep.open: 2`; older releases become backups. |
| A rollback by hand from a backup runs `setup.server` again. |
| A rollback by hand to an open release. |
| A setup failure never switches, and `failed` runs. |
| An automatic rollback runs the rollback commands first, with `SHIPDECK_PREVIOUS`. |
| `rollback.mode: off` keeps the failed release (exit 4). |
| Every runner, local and on the server. |
| A missing server runner stops preflight. |
| A deploy file named by its full path. |
| `-C` with a relative `-c`, run from another folder. |
| `shipdeck tree` matches the live release. |
| `--passwordless` logs in through the instabox gateway. |

### Open findings from the test

#### P1: every first deploy to a new server fails

**Where:** [preflight.rs:73](../../crates/engine/src/deploy/preflight.rs#L73).

Preflight takes the project lock inside the server folder before line 86
creates the folder. The run stops with "opening the project directory: No
such file or directory". The code before format 2 created the folder first.
The test worked around it by creating the folder in advance.

#### P1: a relative deploy file argument fails

**Where:** [naming.rs:76-80](../../apps/cli/src/naming.rs#L76-L80).

`shipdeck releases deploy/production.yaml` says "has no deploy the engine
could read". The same file by its full path works. The CLI compares the
path as typed with the engine's absolute path.

#### P2: "nothing changed" never triggers for a systemd app

**Where:** [content.rs:84](../../crates/engine/src/bundle/content.rs#L84).

The fingerprint includes `.shipdeck/<process>.service`, and that file holds
the release name and version. Two releases with the same files therefore
differ, and a deploy that should exit 6 deploys again. The code before
format 2 fingerprinted a unit file without the release name in it.

#### P3: the built-in unit names an old key

The header of every generated unit says "change service.template instead".
The key is now `process.systemd.template`.

### Resolution

#### P1: every first deploy to a new server fails

Preflight now creates the project folder first, then takes the lock, then
reads the live facts again (`preflight.rs`). A rollback to a missing folder
now says the folder does not exist. No unit test: the step needs a server.
End to end: scenario 1 passes with no pre-created folder.

#### P1: a relative deploy file argument fails

The CLI resolves the file against the project folder (so `-C` counts)
before it matches the engine's deploy. Tests:
`a_relative_deploy_file_matches_the_engine_s_absolute_one` (also with a
`-C`-style folder). End to end: scenarios 15 and 16 pass.

#### P2: "nothing changed" never triggers for a systemd app

The plan keeps the hash of each unit and PM2 file rendered for a neutral
release, and the fingerprint uses it. The same bug had a second cause:
preflight read the live `release.json` from the release root, but format 2
keeps it in `.shipdeck/`. Tests: `a_new_release_name_alone_keeps_the_fingerprint`
(systemd and PM2) and `a_changed_systemd_option_changes_the_fingerprint`.
End to end: scenario 4 passes (exit 6).

#### P3: the built-in unit names an old key

This finding was wrong. No code reads `default.service`. Generated units come
from `bundle/unit.rs`, which already names `process.systemd.template`. The
comment in `default.service` was updated anyway.

### Still open after the fixes

- **P2: the no-agent release list reads the old path.**
  [remote/releases.rs](../../crates/engine/src/remote/releases.rs), in
  `list` around line 290, reads `$d/release.json` from the release root.
  Format 2 keeps it in `.shipdeck/`. The end-to-end test always used the
  agent, so this path was not run.
- **P3: `default.service` is unused.** It also holds old placeholders such as
  `{{service}}` and `{{id}}`. Delete it, or wire it in. *Resolved: deleted.*
- **P3: `unreachable!` in the CLI.** `naming.rs` `slug_of` panics on a case
  that cannot happen. Have `Target::resolved` return the path itself for a
  file, so the type rules the case out.
- **P3: the old key in a feature page.**
  [deployment-config.md](../feature/deployment-config.md) line 589 still says
  `service.template`. *Resolved: the section, which only described the deleted
  file, is removed.*

### Seen, not a finding

Moving an app from systemd to PM2 stops at preflight: "the systemd service
… is still running and would clash with PM2; stop it first". This is the
behaviour from before format 2, and the message says what to do.

## Limits

These were not run in this review:

- the Podman SSH suite, `tests/monorepo_ssh.py`;
- `just build-doc`.

Not read line by line:

- the TUI;
- the protocol;
- the website pages.

The findings above come from the engine, the CLI and the checks.
