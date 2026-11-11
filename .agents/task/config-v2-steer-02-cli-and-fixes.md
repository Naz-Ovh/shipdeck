# Steer 2: the root file, the CLI, and the review fixes

Created: 2026-11-11. A change to the task in
[config-v2-rewrite.md](config-v2-rewrite.md). It wins over the task and over
anything built before it.

It has two parts:

- **Part A** is a design change: the root file and the command line.
- **Part B** fixes the findings that are still open in
  [config-v2-rewrite-review.md](../../docs/review/config-v2-rewrite-review.md).

Read these first:

- the review, both sections;
- [config-v2.md](../../docs/feature/config-v2.md): "Files" again, which is new;
- "Paths on your PC", and the `{{CONFIG}}` row of the placeholder table.

## How this work is checked

The last report said placeholders were fixed in commands. They were not. So
from now on:

1. **A test proves each fix.** Every item below names the test it needs. A
   fix without a test that fails before it and passes after it is not done.
2. **The report lists, for each item:** the files changed, the test name,
   and how you ran it.
3. **Never write "fixed" or "pass" for something you did not run.** Write
   "not run", and why.

## Part A: the root file and the command line

### A1. The root file

The project file is now called the **root file**. Use that name in code,
messages and docs.

- It must hold `root: true`. Without it, a file is not a root file.
- `deploys.<name>` is either:
  - a deploy file's path, ending in `.yaml` or `.yml`; or
  - the deploy written in place, as a map.
- A deploy written in place follows every rule of a deploy file:
  - the same type and the same checks;
  - problems point at their line in the root file;
  - its `./x` paths start from the root file's folder;
  - `{{CONFIG}}` is the root file's folder.
- A deploy file never holds `root` or `deploys`. Those are unknown keys
  there, as today.
- **Deploy names:** letters, digits, `-` and `_` only. Remove `.` from the
  deploy-name rule.
- **Schema:**
  - `root` is the constant `true`, and required;
  - each `deploys` value is a string or a deploy file;
  - `just schema` writes it.

### A2. The command line

**`--cwd <dir>`, short `-C`:**

- Sets the project folder.
- Applies first, as in `git -C`: every relative path on the command line,
  including `-c` and a deploy file argument, starts from it.
- Use one folder value for the whole run. Do not call
  `std::env::set_current_dir`, and never read the process's current folder
  after the start.

**`--config <file>`, short `-c`:**

- The root file to read, instead of `shipdeck.yaml` or `.shipdeck` in the
  project folder.
- It always names a file, so it needs no ending; `.shipdeck` is accepted.
- It does not change the project folder.

**`--shipdeckfile` is removed.**

**The `<deploy>` argument** of every command that takes one (`deploy`,
`rollback`, `releases`, `server-info`, `work`, `events`, `tree` and the rest):

| Written | Read as |
| --- | --- |
| Ends in `.yaml` or `.yml` | A deploy file. No root file is read. |
| A plain name | A deploy name from the root file. |
| Anything else, such as `./production` | An error: a deploy file ends in `.yaml` or `.yml`. |

- **Only the ending decides.** Do not check for `/`, and never try one
  reading after the other fails.
- **Mixing:** `shipdeck deploy api ./staging.yaml` runs both, in order, as
  `deploy` does today with several names.
- **Model it as a type.** One enum, `Target { Name(String), File(PathBuf) }`,
  parsed once from the argument. Every command takes the enum. Do not write a
  helper that each command calls on its own.

### A3. Errors

| Case | The message says |
| --- | --- |
| No root file, and no deploy file named | The folder, and the two root file names it expected. Not "file has 1 problem". |
| `-c` names a file without `root: true` | "This is a deploy file; run `shipdeck deploy <file>`." |
| A deploy file argument holds `root: true` | "This is a root file; use `-c <file>`." |
| A name the root file does not list | The root file's path, the names it lists, and that a file needs `.yaml` or `.yml`. |
| A file that does not exist | Its full path. |

### A4. TUI and init

- The TUI lists the root file's deploys, including deploys written in place.
- `shipdeck init` writes a root file with `root: true`.

### A5. Tests for part A

In the sibling `_tests.rs` files:

- the root file's checks: `root` missing, `root: false`, both names, and a
  deploy written in place with a problem pointing at its line;
- a deploy name with a dot is refused;
- argument parsing: a name, a `.yaml` file, a `.yml` file, `./production`
  (error), and a mixed list;
- `-C` with a relative `-c` and a relative deploy file: both start from the
  `-C` folder;
- a deploy written in place: `./x` and `{{CONFIG}}` start from the root
  file's folder;
- every error in A3.

### A6. Docs and examples

- Every `examples/*/shipdeck.yaml` gains `root: true`. Add one example with a
  deploy written in place.
- Update the CLI manual (`apps/cli/manual/`), the website pages, and
  `docs/code/config.md` for the root file, `-c`, `-C` and the `<deploy>`
  argument.
- Remove every `--shipdeckfile` and "project file" from code, docs and
  messages:

  ```sh
  grep -rn -i "shipdeckfile\|project file" --include='*.rs' --include='*.md' --include='*.mdx' --include='*.yaml' .
  ```

  This must find nothing outside `.agents/` and `docs/review/`.

## Part B: open review findings

Each item names where it is, what to do, and the test it needs.

### B1. Placeholders in commands and `env` (P1)

**The bug:** `{{NAME}}` reaches the shell as plain text. Reproduce it first.
In a scratch project, a build command `echo "{{VERSION}}" > out.txt`, run
with `shipdeck tree <deploy> --build`, writes `{{VERSION}}`.

**Fix:**

- Expand with `placeholders::expand` and the step's `Values` before a command
  runs:
  - local commands (`build::run_command`);
  - server commands (`commands::server_line`);
  - the code of a runner command;
  - `process.env` values;
  - the PM2 file.
- Add no quotes; the design says so.
- **Tests:**
  - a local command whose output file holds the expanded value;
  - `server_line` for an `sh` command and for a runner command;
  - an `env` value;
  - the PM2 file.

### B2. An automatic rollback runs the rollback commands (P1)

**Where:** `recover.rs`, in `recover` and `restore_back`.

**Fix:**

- Before the restore, run `rollback.server` and then `rollback.local`, from
  the current deploy file, as `step_put_back` does for a rollback by hand.
- In those commands, `SHIPDECK_PREVIOUS` names the release being put back.
- **Test:** the automatic path runs the rollback commands before the switch
  back, for `mode: auto` and for an `ask` answered yes.

### B3. `--on-failure` overrides `rollback.mode` only when given (P1)

**Fix:**

- Make the flag an `Option` with no default.
- Send `on_failure: None` when it is not given.
- **Test:** with no flag, the request carries `None`, so `rollback.mode: off`
  in the file keeps the failed release.

### B4. Unused code (P2)

**Fix:**

- Delete the 7 functions behind `#[allow(dead_code)]`, in `activate.rs`,
  `build.rs`, `recover.rs` and `preflight.rs`.
- Remove the parameter or variable that each of these lines silences. Don't
  silence it another way, such as with a `_` prefix on a parameter nobody
  passes on purpose:

  | File | Line |
  | --- | --- |
  | `deploy/run.rs` | `let _ = name;` |
  | `deploy/build.rs` | `let _ = name;` |
  | `deploy/activate.rs` | `let _ = server;` |
  | `deploy/recover.rs` | `let _ = server;` |
  | `deploy/preflight.rs` | `let _ = (step, scope.app);` |
  | `deploy/backup.rs` | `let _ = &mut hits;` |
  | `tree.rs` | `let _ = index;` |
  | `config/rules.rs` | `let _ = &self.deploy.release.steps;`, `let _ = &self.deploy.apps;`, `let _ = refs;` |
  | `config/rules/file_rules.rs` | `let _ = (step, side, list);`, `let _ = list;` |
  | `config/rules/app_rules.rs` | `let _ = app;`, `let _ = base;` (twice) |

- Remove the copy at `bundle/plan.rs` `generated()`, which takes
  `entry.path` out and puts it back.
- **Check:** this grep finds none of the lines above. `let _ = writeln!(…)`,
  `let _ = x.send(…)` and `.await` lines are fine; leave them.

  ```sh
  grep -rnE "allow\(dead_code\)|let _ = &?(mut )?\(?[a-z_][a-z0-9_.]*(, ?[a-z_][a-z0-9_.]*)*\)?;" crates/engine/src apps/cli/src --include='*.rs'
  ```

### B5. The process manager as one choice (P2)

**Fix:**

- After resolve, the pipeline sees one enum that carries its block:
  `Manager::Systemd(&Systemd)`, `Manager::Pm2(&Pm2)`, `Manager::Oxmgr(&Oxmgr)`.
- If serde cannot parse the YAML shape straight into an enum, keep the three
  fields only in the written type. Then convert them in one place in
  `resolve.rs`. Nothing after resolve reads the three fields.
- **Test:** the conversion.

### B6. `{ path }` without `to` (P2)

**Fix:**

- Make `MappedFile.to` optional, for `binary` and for `files`.
- `{ path: x }` behaves like `x`. The existing rule still applies: a path
  outside the project or the app's source needs `to`.
- **Tests:** `binary: { path: x }`, and a `files` entry `{ path: x }`.

### B7. `deploy_completed` is recorded once (P3)

**Fix:**

- Remove one of the two calls: in `activate.rs` `step_done`, or in
  `run.rs` `pipeline`.
- **Test:** one event per deploy.

### B8. No placeholder nginx or oxmgr files (P3)

**Fix:**

- `bundle/plan.rs` stops adding `.shipdeck/nginx/*.conf` and
  `.shipdeck/oxfile.toml` until those parts are built.
- **Test:** `shipdeck tree` on a deploy with `nginx:` lists neither.

### B9. Full paths in file errors (P3)

**Fix:**

- "The build did not produce …" and every "does not exist" error on your PC
  show the full path.
- **Test:** one of them.

## When it is done

1. Run every check in task section 9. Write each result as you saw it.
2. Rewrite the report, `.agents/task/config-v2-rewrite-report.md`, with all
   8 parts that task section 10 asks for. Add:
   - a table for this steer: item, files, test name, result;
   - the list of deleted tests, each with the new test that covers it, or
     the reason it was dropped.
3. Update `.agents/handoff.md`.
4. Do not commit.
