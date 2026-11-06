# Code layout

Where the code sits, and where new code goes. Shipdeck is one Cargo
workspace: the engine does the work, the frontends show what it reports.

## Crates

- `crates/engine/` (`shipdeck-engine`): `config` (deploy.yaml: types that
  are the JSON Schema, the line-aware YAML tree, the launch check, globs),
  `build` (version, local commands, ELF checks), `bundle` (picking files,
  their hashes, the fingerprint and the receipt, tar.zst, unit), `ssh` (sessions,
  login, host keys, `~/.ssh/config` aliases), `remote` (server scripts; the backup and which files go in it;
  the delta and its check at unpack; the service by hand),
  `release` (ids, release.json), `setup` (the server-setup.sh URL),
  `deploy` (the deploy and rollback flows, `report` for progress),
  `init` (the presets and the new deployable folder), `rpc` (the engine
  process: the loop, the methods, and what it reports).
- `crates/tui/` (`shipdeck-tui`): the screens, dialogs and key handling.
- `crates/headless/` (`shipdeck-headless`): the `deploy`, `rollback` and
  `releases` commands for scripts and CI: plain lines, no questions, exit
  codes.
- `crates/protocol/` (`shipdeck-protocol`): every message between the engine
  and an app, the JSON Schema made from those types, and the connection both
  ends use. No engine logic lives here. See [protocol.md](protocol.md).
- `apps/cli/` (package `shipdeck`, the name the release files keep): the
  flags (clap lives only here), the login, the config check, then hands over
  to the TUI or headless. `init`'s questions live here too (`inquire`), and
  `man`, whose text is `apps/cli/manual/` (static data via
  `apps/cli/build.rs`). The TUI and headless only show what the engine
  reports; neither depends on the other.

## Where new code goes

- The work belongs to the engine. A frontend only shows what the engine
  reports, and never grows its own copy of the logic.
- A new subject is a new folder module in its crate, named after the subject.
  It never becomes a `utils` box.
- The move to `apps/` and `crates/` is done (`p1a2`), the protocol crate is
  done (`p1b1`), and so is `shipdeck engine` (`p1c1`). Next the TUI becomes one
  of its apps (`p1d2`); until then it still calls the engine in-process, and
  both ways reach the same `deploy` and `remote` code.
