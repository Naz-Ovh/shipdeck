# Reaching a server

Every value and every command that crosses SSH has one proper form. Break one
of these and a deploy fails on someone's server, or opens a hole.

## Values in a shell

- Every value that reaches a remote shell is either validated in
  `config/rules.rs`, checked with `release::is_safe_id`, or quoted with
  `ssh::sh_quote`. Keep it that way.
- The one exception is the `server:` commands in a deploy file: shell by design,
  written by whoever deploys and run as the deploy user.

## Remote scripts

- Remote scripts are POSIX `sh`; they run through `sh -c`, whatever the login
  shell, after the variables the login shell's startup files export
  (`ssh::shell_env`, off with `ssh.shell_env: false` or `--no-shell-env`).

## Sudo: one rule

- The SSH user is root, or `sudo -n -- sh -c 'id -u'` answers `0` (full
  password-free sudo). `agent::layout::sudo_prefix` checks it once, when the
  agent starts, and its error says how to grant it.
- After that check, a command that needs root is run with `sudo -n` (the
  `Server::sudo` prefix, empty for root). There is no per-command sudo rule to
  keep in step, and no setup script.
