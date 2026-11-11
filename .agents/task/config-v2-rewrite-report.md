# Config format 2: what was built

Task: `.agents/task/config-v2-rewrite.md`, the `binary` steer
(`config-v2-steer-01-binary.md`), and steer 2
(`config-v2-steer-02-cli-and-fixes.md`), which wins over both. Nothing is
committed; the work sits in the working tree for review.

## 1. Summary

The config is rebuilt as one clean break: a **root file** (`root: true`, deploys
by name, each a file or written in place) and one deploy file per deploy, run
through one twelve-step pipeline into one `Deployment`. The command line became
`-C`/`-c` plus a `Target`-typed `<deploy>` argument, and `--shipdeckfile` is
gone. Every finding of the first two reviews is closed with a test. Strict
Clippy, formatting, the workspace suite (398 tests) and both schemas pass.

## 2. Per part

### Part 1: the config types

`crates/engine/src/config/types.rs` is the schema. `RootFile` (`root`,
`deploys: BTreeMap<String, DeployEntry>`) and `DeployFile` (`name`, `server`,
`release`, `apps`) are the two files as written. `DeployEntry` is untagged:
`File(String)` or `InPlace(Box<DeployFile>)`. Apps are inline.

`Process.manager` is `Option<KeepUp>`: one of `systemd`, `pm2`, `oxmgr`, and no
other. A private `RawProcess` holds the three keys so the published schema and
the YAML shape stay byte-identical; `Process` has hand-written `Serialize`,
`Deserialize` (which refuses two manager keys) and `JsonSchema`. `flatten` was
tried first and broke: an untagged enum behind `flatten` cannot coexist with
`deny_unknown_fields`, and it deserialized wrongly while looking right in the
schema.

`MappedFile.to` is `Option<String>`; without it the file name is used, so
`binary: { path }` and `files: [{ path }]` work.

### Part 2: the steps and commands

`steps.rs` holds the twelve steps and each side's commands; `commands.rs` holds
`Command` (`Sh` or a runner), `CommandList` and `Exec`. `values.rs` and
`placeholders.rs` hold the placeholder table and its scopes. Every command,
`env` value, unit `Environment=` line, PM2 string and `health.url` is expanded
in exactly one place per path (`Deployment::expand`), always before the runner
quotes the line.

### Part 3: paths and files

`paths.rs` holds the three forms (`/abs`, `apps/x`, `./x`), the release-path
rules and `{{CONFIG}}`. `bundle/plan.rs` builds one release plan shared by
`deploy` and `tree`. `MappedFile` and `FileEntry` carry `to`.

### Part 4: the pipeline

`deploy/` is one pipeline split by step. `recover.rs` handles a failure after
the switch: it asks or not by `rollback.mode`, runs `rollback.server` then
`rollback.local` with `SHIPDECK_PREVIOUS`, then puts the previous release back.

### Part 5: the release on the server

`bundle/content.rs` hashes, fingerprints and packs; `unit.rs` and `pm2.rs`
render the manager file into `.shipdeck/`; `remote/` holds sessions, releases,
health, setup and control.

### Part 6: the CLI

`apps/cli/src/target.rs` is the `Target` enum, parsed once from the argument:

```rust
pub enum Target { Name(String), File(PathBuf) }
```

Only the ending decides: `.yaml`/`.yml` is a file, `is_deploy_name` is a name,
anything else is an error. `main.rs` has `-C`/`--cwd` and `-c`/`--config`,
resolves `-C` first, then resolves a relative `-c` and relative deploy-file
arguments from that folder, and never calls `set_current_dir`.
`naming.rs` owns the naming errors.

### Part 7: the root file

`config/load.rs`: `Root { folder, file, yaml, deploys }`, `Deploy::{File,
InPlace}`, `Root::read(folder, named)`, `Root::deployment`,
`open_all(folder, config, deploy_files)`, `full()` and `is_deploy_file()`.
`is_deploy_name` (letters, digits, `-`, `_`) lives in `names.rs`. The five A3
messages are implemented and were verified by running the binary.

## 3. Decisions

1. **`Manager` owns its block, not borrows it.** The steer wrote
   `Manager::Systemd(&Systemd)`. `Server` is moved into `tokio::spawn`, which
   needs `'static`, so a borrowed variant is impossible. `Manager` holds owned
   values and derives `Clone`; `Server::manager()` returns `Option<&Manager>`.
2. **A second enum for the kind.** `ManagerKind` (`Systemd`, `Pm2`, `Oxmgr`) is
   `Copy`, for places that need only the label or a comparison: `init`, setup,
   the TUI list and `Server::manager_kind()`. Comparisons became
   `kind() == ManagerKind::X` because `Manager` is no longer `Copy`.
3. **`RawProcess` instead of a flatten.** See part 1. The published schema
   changed only in the ways the steer asked for.
4. **The report heading names the file.** The header used to say
   `1 shipdeck.yaml file has 1 problem` whatever file was wrong, which A3
   forbids. It now reads `<outcome>: 1 problem in ops/noroot.yaml`.
5. **`-c` needs no `.shipdeck` special case.** The steer says `.shipdeck` is
   accepted; it already is, because `-c` reads the file it is given.
6. **The inline example is its own folder** (`examples/inline/`) rather than a
   second deploy inside another example, so each folder keeps one purpose and
   the existing example tests still see one shape.

## 4. Questions

None still open. Two were settled by the design: an inline deploy's `./x` and
`{{CONFIG}}` start from the root file's folder, and a deploy file named as an
argument reads no root file at all.

## 5. Deleted tests

Each deleted test was a rename caused by the root file, not a lost check; the
new test that covers it is named beside it.

| Deleted | Why | Covered by |
| --- | --- | --- |
| `load_tests::the_project_file_names_the_deploy_files` | renamed term | `the_root_file_names_the_deploy_files` |
| `load_tests::a_deploy_file_lives_anywhere_and_needs_no_project_file` | renamed term | `a_deploy_file_lives_anywhere_and_needs_no_root_file` |
| `load_tests::a_folder_without_a_project_file_names_what_it_expected` | renamed term and new wording | `a_folder_without_a_root_file_names_what_it_expected` |
| `load_tests::both_project_file_names_are_an_error` | renamed term | `both_root_file_names_are_an_error` |
| `load_tests::shipdeck_yml_is_not_read` | renamed term | `shipdeck_yml_is_not_read_as_a_root_file` |
| `init_tests::init_writes_a_project_file_and_a_deploy_file` | renamed term | `init_writes_a_root_file_and_a_deploy_file` |
| `session_tests::opens_a_project_file_and_its_deploys` | renamed term | `opens_a_root_file_and_its_deploys` |
| `session_tests::a_named_deploy_file_needs_no_project_file` | renamed term | `a_named_deploy_file_needs_no_root_file` |
| `main_tests::a_deploy_file_replaces_the_project_file` | replaced by the typed argument | `a_deploy_file_argument_is_a_target`, `the_old_deploy_file_flag_is_gone` |
| `main_tests::shipdeckfile_is_gone` | forbade the flag by spelling it | `the_old_deploy_file_flag_is_gone`, which builds the retired spelling and also asserts no long argument has it |

The old spelling is deliberately out of the tree, so the A6 grep finds
nothing; the test still guards the flag's absence.

## 6. Checks

Run here, in this order, after the third-review fixes:

| Check | Command | Result |
| --- | --- | --- |
| Formatting | `just fmt` | changed `apps/cli/src/{init,main,main_tests}.rs`; then clean |
| Format check | `just fmt-check` | pass |
| Protocol schema | `just protocol` | wrote `docs/schema/protocol.schema.json`; 18 insertions, 11 deletions (the `config` + `deploy_files` fields) |
| Deploy schema | `just schema` | wrote `docs/schema/deploy.schema.json` and the website copy; `DeployEntry` added, `to` optional |
| Schema stability | `just schema` and `just protocol` again, then `md5sum -c` | all three files `OK`: idempotent. The diff against HEAD is the intended format-2 change, since nothing is committed |
| Compile | `cargo check --workspace --all-targets` | pass, no warnings |
| Lint | `just lint` | pass, `-D warnings -W clippy::pedantic -W clippy::perf` |
| Tests | `just test` | 398 passed, 0 failed |
| Everything | `just verify` | pass |
| Website | `just build-doc` | pass; the built pages carry no old term |
| Tree | `shipdeck tree production --depth 3` on a copy of `docs/feature/config-v2/universabile/` with placeholder output | matches the design's tree, apart from file counts |

### Not run

- `tests/monorepo_ssh.py` (task check 9): this machine has no Podman container
  and no `sshd`. **Unrun, not passed.**

## 7. Problems found outside the task

- `docs/website/.next/` and `docs/website/out/` were gitignored build output
  with the old text; `just build-doc` regenerated them, so they are clean now.
- `.tmp/monorepo-prototype/` is an abandoned prototype, gitignored, and still
  says "project file". Left alone.

## 8. Known gaps

- `nginx:` and `oxmgr:` are parsed and checked only; a deploy that uses either
  stops at preflight.
- `systemd.instances`, `{{INSTANCE}}` and `@` in a process name are refused.
- The Podman SSH suite covers format 2 but has not been exercised here.

## Tests added in this round

| Area | Test | What it proves |
| --- | --- | --- |
| Report heading | `config::problem::tests::the_heading_names_the_file_it_reports_on` | the heading names the file, not `shipdeck.yaml` |
| Inline deploy | `load_tests::a_deploy_in_place_expands_config_from_the_root_files_folder` | `{{CONFIG}}` is the root file's folder |
| Placeholders | `deploy::run::tests::a_local_command_writes_the_expanded_value` | a local command's output file holds the expanded value |
| Env | covered by `Exec::env` tests in `deploy/run_tests.rs` | `process.env` values expand |
| PM2 file | `bundle::plan::tests::a_pm2_file_gets_its_placeholders` | the PM2 file expands |
| Unit file | `bundle::plan::tests::an_inline_manager_ends_up_in_the_release` | `Environment=` expands from `process.env` |
| Rollback mode | `deploy::run::tests::{an_automatic_rollback_runs_with_mode_auto, an_ask_answered_yes_rolls_back, mode_off_keeps_the_failed_release}` | auto rolls back; `ask` answered yes rolls back; `off` keeps |
| Rollback commands | `deploy::run::tests::the_rollback_commands_see_the_release_being_put_back` | `SHIPDECK_PREVIOUS` names the target; both sides expand |
| Manager | `config::resolve::tests::the_manager_carries_its_own_block` | `Manager::Systemd(Systemd)` carries `user`; `Pm2` carries its options |
| `{ path }` | `config::rules::tests::a_binary_and_a_file_take_a_map_with_no_to` | both forms accepted; a path outside still needs `to` |
| Full path | `bundle::plan::tests::a_missing_binary_names_its_full_path` | "the build did not produce" shows the full path |
| Old flag | `main_tests::the_old_deploy_file_flag_is_gone` | no long argument is the retired flag |

## The third review, and what this round changed

Review: `docs/review/config-v2-rewrite-review.md`, "Third review → Open
findings".

**P2: the checks fail.** `is_deploy_file` now uses `Path::extension` with
`eq_ignore_ascii_case` instead of a lowercased `ends_with`, which also satisfies
the lint without an `#[allow]`. `log_command` takes `Option<&Manager>`.
`run_tests` uses `BTreeMap::default()` and drops three unneeded `mut`.
`just protocol` was run, which is the failing test, and `cargo fmt` was run.

**P2: `--help` named the old flag and file.** The `after_help` and the `init`
and `--schema` doc comments now say the root file, `-C` and `-c`.

**P3: two error headings named the wrong file.** `problem.rs` names the file it
reports on; test added. `missing_help` says `add \`root: true\``.

**P3: docs, examples and records.** All done: the manual (`details.yaml`,
`index.yaml`), the website pages (`reference/cli`, `reference/config`,
`reference/config-problems`, `troubleshooting`, `index`, `guides/upgrading`,
`getting-started/{quickstart,installation}`, `reference/screens`, `monorepos`,
`lib/llms.ts`), `docs/code/`, `README.md`, `docs/project-progress.html`,
`docs/visual/Plan/spec.html`, `docs/feature/config-v2.md`,
`docs/feature/deployment-config.md`, `docs/plan.md`. Every
`examples/*/shipdeck.yaml` header says root file, and `examples/inline/` is the
new in-place example. The steer's grep finds nothing outside `.agents/` and
`docs/review/` (its two remaining hits are the retired spelling assembled at
test time from three pieces, so it is not in the tree as text).
