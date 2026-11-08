# Config: shipdeck.yaml, the schema, the manual, the examples

Everything people read or write about `shipdeck.yaml` moves together. A change
in one usually touches all four.

## The schema

- The config types in `crates/engine/src/config` are the schema; there is no
  second definition to keep in step.
- Every field needs a doc comment: editors show it in the JSON Schema.
- After a change run `just schema`. It writes `docs/schema/deploy.schema.json` and the
  website's copy in `docs/website/public/`, and a test fails until you do.

## Errors

- A config mistake is a `config::Problem` with the line it is on and a hint,
  never a bare string. Shipdeck reports them all before it starts.

## The manual

- `apps/cli/manual/` is for people using shipdeck and the agents helping
  them: setup, use, troubleshooting. No code internals.
- A change in behaviour updates its page.
- `configs.yaml` must name exactly the schema's properties (`man_test.rs`
  checks).

## The examples

- `examples/<kind>/shipdeck.yaml` must keep parsing and start with the schema
  line for this version (`schema_test.rs` checks).
- A new field goes into the README table, an example, and
  `apps/cli/manual/configs.yaml`.
