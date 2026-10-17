# Changelog

## 0.6.1 (2026-10-17)

### Upgrading from 0.6.0

- Change the schema line in your `deploy.yaml` files to `v0.6.1`.

### Changed

- **Any PM2 version deploys.** Preflight used to refuse a PM2 newer than
  6.x. shipdeck uses only PM2's basic commands, so it now takes any
  version and only warns when it is newer than the tested 7.x.

### Docs

- **How tools are found on the server.** `shipdeck man details shell-env`
  and the README list which startup files each login shell reads (bash
  reads `~/.bashrc` only through `~/.profile` or `~/.bash_profile`) and
  give the command that sees what shipdeck sees:
  `ssh deploy@prod '"$SHELL" -i -l -c "command -v pm2; pm2 -v"'`. A
  plain `ssh deploy@prod pm2 -v` can say "not found" while shipdeck
  finds pm2.
- **When pm2 is not found.** Troubleshooting now explains the
  ``"`pm2 -v` did not print a version"`` error and its usual causes: pm2
  installed for another user, a `~/.bash_profile` that skips `~/.bashrc`
  (and nvm with it), or `ssh.shell_env: false`.
- **PM2 7** needs Node.js 18 or newer; the tested release is
  `pm2@7.0.4`. After upgrading PM2, run `pm2 update` as the deploy user.

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
