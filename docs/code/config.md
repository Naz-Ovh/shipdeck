# Config: the root file, the deploy file, the schema, the manual, the examples

Everything people read or write about config format 2 moves together. A change in one usually
touches all four.

## The types are the schema

- The config types in `crates/engine/src/config` are the schema; there is no second definition
  to keep in step.
- `types.rs` holds the two files as written: `RootFile` and `DeployFile` with `Server`,
  `Release`, `App`, `Process`, the managers, `Nginx`, `FileEntry`, `Command` and the step
  structs.
- `placeholders.rs` holds every `{{NAME}}`, where it has a value, and the environment names.
- `paths.rs` holds the three path forms and the release-path rules.
- `resolve.rs` holds `Deployment`: the deploy file with its paths resolved, and the methods the
  pipeline uses (`source`, `start`, `binary_place`, `keep`, `rollback_mode`, ...).
- Every field needs a doc comment: editors show it in the JSON Schema. After a change run
  `just schema`; it writes `docs/schema/deploy.schema.json` and the website's copy, and a test
  fails until you do.

## Reading a file

Reading goes in three passes, so every problem in a file is reported at once:

1. `yaml.rs` parses into a tree that keeps each key's line and column.
2. `check.rs` walks that tree against the generated JSON Schema: unknown keys, wrong types, and
   a command map read in one place so its message is the only one shown.
3. `rules.rs` checks the values: required and refused keys per app kind, one manager, reserved
   properties, names, release paths, and every placeholder in its scope.

`load.rs` reads the root file (or the files named on the command line), and turns a clean
deploy file plus its endpoint into a `Deployment`. Only `Deployment` reaches the pipeline.

## Errors

- A config mistake is a `config::Problem` with the line it is on and a hint, never a bare
  string. Shipdeck reports them all before it starts.

## The manual

- `apps/cli/manual/` is for people using shipdeck and the agents helping them: setup, use,
  troubleshooting. No code internals.
- A change in behaviour updates its page.
- `configs.yaml` must name exactly the schema's properties (`man_test.rs` checks). It is
  generated from the schema when the schema changes.

## The examples

- `examples/<kind>/` holds a root file (`shipdeck.yaml`) and a deploy file (`deploy.yaml`).
- `examples/inline/` writes one deploy in the root file itself, beside one in a deploy file.
- A test reads every example, so a change to the format breaks it there first.
- A new field goes into the README table, an example, and `apps/cli/manual/configs.yaml`.
