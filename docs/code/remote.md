# Reaching a server

Every value and every command that crosses SSH has one proper form. Break one
of these and a deploy fails on someone's server, or opens a hole.

## Values in a shell

- Every value that reaches a remote shell is either validated in
  `config/rules.rs`, checked with `release::is_safe_id`, or quoted with
  `ssh::sh_quote`. Keep it that way.
- The one exception is the `server` commands in `pipeline`: shell by design,
  written by whoever deploys and run as the deploy user.

## Remote scripts

- Remote scripts are POSIX `sh`; they run through `sh -c`, whatever the login
  shell, after the variables the login shell's startup files export
  (`ssh::shell_env`, off with `ssh.shell_env: false` or `--no-shell-env`).

## Sudo: a contract with every installed server

- The sudo commands in `remote::sudo_commands` and
  `remote::control_commands` are a contract with the sudoers line people
  install. Changing one breaks every existing server, and `server-setup.sh`
  must change with it (`setup_test.rs` checks they match).
- Preflight requires all of `sudo_commands`, so a command a deploy can do
  without goes in `control_commands`, and older servers keep deploying.
- `scripts/server-setup.sh` is public and runs as root through
  `curl | bash`, pinned to a release tag. It validates every argument before
  doing anything; keep it that way, and tag every release (`v<version>`).
