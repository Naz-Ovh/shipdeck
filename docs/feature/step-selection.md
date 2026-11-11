# Run only some steps, or skip some (future plan)

Status: **future plan**. Nothing is built. Tracker row: `p8c5`.
Owner's idea, 2026-11-11.

## The problem

A deploy always runs all 12 steps of
[config format 2](config-v2.md#steps). Sometimes you want less:

- rewrite nginx, and nothing else;
- upload a release now, and make it live later;
- deploy without a backup, because the disk is nearly full.

Today the only way is to edit the config for one run.

## What the user gets

The user picks steps for **one run**. The config does not change.

| Where | How |
| --- | --- |
| CLI, by hand or in CI | `--only <steps>` or `--skip <steps>`, for example `--skip backup,health`. |
| TUI | Before a deploy starts, a list of the 12 steps with a tick box each. |
| Desktop app and web dashboard, later | The same list as the TUI. |

## The design

### One rule, in the engine

Every front end sends the same thing to the engine: the steps to run. The
engine checks the choice and refuses a bad one with a clear message.

The CLI, the TUI and the later UI never check the rules themselves. So they
can't disagree.

The TUI and the UI also need to grey out a box and say why. For that, the
engine answers one more request: "for this deploy, which steps can be picked,
and what does each one need?"

### Skipping a step skips all of it

A step has two parts: your commands, then shipdeck's action. Skipping a step
skips both parts. One step name always means the whole step.

### Steps that depend on others

| Step | Rule |
| --- | --- |
| `preflight` | Always runs. It takes the deploy lock and checks the connection. |
| `build` | Needed by `upload`, because it makes the file list that `upload` sends. |
| `upload` | Needs `build`. Makes the new release folder. |
| `setup` | Needs `upload`. It runs in the new release. |
| `backup` | Can be skipped. Then this release has no backup file. |
| `switch` | Needs `upload`. Skipping it gives a "stage only" deploy. |
| `restart`, `nginx`, `health`, `done` | Run on the new release after `switch`, or on the live release. See the two kinds of run below. |
| `rollback`, `failed` | Not picked. They run when something fails, as today. |

A choice that breaks a rule is an error, never a silent broken deploy. For
example, `--skip build` alone is an error, because `upload` needs it.

### Two kinds of run

The engine decides the kind from the steps picked:

1. **A new release.** The choice includes `build` and `upload`. Steps run
   as today, minus the skipped ones.
2. **Work on the live release.** The choice holds only steps from
   `restart`, `nginx`, `health` and `done`. No release is made. For example,
   `--only nginx` rewrites nginx for the live release.

A choice that mixes the two in a way that has no meaning is an error. For
example, `--only setup` has no new release to run in.

### Stage only

`--skip switch` uploads the new release but does not make it live. Then
`restart`, `nginx`, `health` and `done` don't run either, because there is
nothing new to run them on. Asking for one of them by name together with
`--skip switch` is an error.

### Safety

- **No `health`:** a failed health check can't start a rollback any more.
  The output says so before the deploy starts. A failure in `switch`,
  `restart` or `nginx` still starts one.
- **No `backup`:** the output says that this release has no backup file.
- **The release record** lists the skipped steps. `shipdeck releases` and the
  release details show that the release is partial.

## Why not the other ways

- **A setting in the config, like `steps.backup.enabled: false`:** a skip is
  a choice for one run, not part of the project. A config setting would
  also end up in every release's `.shipdeck/settings.json`.
- **Checks in each front end:** the CLI, the TUI and the UI would drift
  apart. One check in the engine stays the truth.
- **Skipping only your commands, or only the action:** it gives one step
  name two meanings. If it is ever needed, it can come later as its own
  flag.

## Tracker steps

- `p8c5`: run only some steps, or skip some. Deferred until the owner says go.

## Open questions

Each has the answer shipdeck takes until the owner says otherwise.

1. **Flag names.** `--only` and `--skip`, taking a comma list, and allowed
   more than once. They can't be used together.
2. **A name for stage only.** No extra flag at first; `--skip switch` is
   enough. A later `shipdeck deploy --stage` is possible.
3. **Make a staged release live later.** Out of scope at first. Today,
   `shipdeck rollback` to that release's id is the nearest thing.
4. **`--only switch` with an already staged release.** An error at first.
   It belongs with question 3.
5. **Skip `rollback` or `failed`.** Not part of this plan. `--on-failure`
   already chooses what happens after a failure.
6. **Remember the TUI's last choice.** No. Every deploy starts with all
   steps ticked, so a skipped step is never skipped by accident.
