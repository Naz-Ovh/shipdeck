# Changelog

## 0.6.0 (2026-10-17)

### Upgrading from 0.5

- **Two keys moved.** `service.keep` is now `pipeline.backup.keep`, and
  `pipeline.health.rollback` is now `pipeline.failed.rollback`. shipdeck
  refuses the old keys and names the new place of each.
- **Servers need `sha256sum`** (from coreutils). Preflight checks for it.
- **Run `server-setup.sh` again on systemd servers** to use `:stop`,
  `:start` and `:logs`. Deploys keep working without it; only those
  commands say what is missing.
- Change the schema line in your `deploy.yaml` files to `v0.6.0`.

### New

- **Only what changed is sent.** A deploy uploads just the files the live
  release lacks; the rest is copied on the server and checked against
  their SHA-256 before the release goes live.
- **Nothing changed, nothing deployed.** A deploy whose files, service
  and `deploy.yaml` equal the live release's asks first in the TUI.
  `shipdeck deploy` stops with exit code `6`; `--allow-unchanged` deploys
  anyway.
- **Smaller backups.** After setup, each release keeps a backup of only
  what it shipped (`pipeline.backup`), not `node_modules` or whatever
  setup makes. A rollback to a backup unpacks it and runs setup again.
- **`:` commands in the TUI.** On the Deployables screen, `:logs`
  follows the service's log live, and `:restart`, `:stop` and `:start`
  run the service by hand.
- **`shipdeck man`**, the manual in the binary, for you and the coding
  agent helping you: `man index` lists the topics, `man configs` every
  `deploy.yaml` property, and `man details <KEY>...` gives them in full.

### Fixed

- Server output with accented characters (file names, logs) could be
  garbled where an SSH packet ended mid-character.
- A file that changes while the release is packed now stops the deploy
  instead of shipping something different from what was hashed.
