# Deployment operations

Detailed reference. Start with the [overview](../../README.md).

## Permissions

### What the user must be able to do

shipdeck deploys as `ssh.user`, whoever that is. On the server, that user
must be able to:

| What | Why |
| --- | --- |
| create `service.dir` (itself, or with sudo), or write to it | the first deploy creates the app's folder; every deploy writes releases into it |
| `systemctl daemon-reload`, `systemctl enable <service>`, `systemctl restart <service>` | install and restart the app |
| `install -m 0644 <dir>/current/<service>.service /etc/systemd/system/<service>.service` | put the release's unit in place |
| `journalctl -u <service> -n 30 --no-pager` | show the log when a release fails |
| `systemctl stop <service>`, `systemctl start <service>`, `journalctl -u <service> -f -n 200 --no-pager` | `:stop`, `:start` and `:logs` in the TUI; a deploy never needs them |

As root these simply run. Any other user runs all but the first row through
`sudo -n`, so it needs sudo without a password for them. Preflight checks
each one a deploy needs and, when something is missing, stops and names it.

## On the server

```
/opt/my-app/
├── current -> releases/v0.3.1-ab12cd3-20261014.153012
├── releases/        newest `keep.open` releases, unpacked
├── backups/         each release's backup, `<id>.tar.zst`, for `keep.open` + `keep.compressed` releases
└── shared/          kept across deploys; the only folder the default unit may write to

/opt/shipdeck/agent/
├── v<version>/agent root-owned; launched by full path with sudo
└── .install.lock    coordinates simultaneous installs

~/.shipdeck/
└── state.db         numbered events for work, events and replay

~/.shipdeck-agent.log the agent's own diagnostics
```

A release id is `v<version>-<git sha>[-dirty]-<UTC yyyymmdd.hhmmss>`. Each
release carries `release.json` (who deployed what, from which commit, and
the fingerprint of what it holds) and its own unit, whose `Description=`
holds the version, so `systemctl status` shows what is live. While a
release is sent, `releases/` also holds its archive, receipt and copy lists
(`.<id>.tar.zst`, `.<id>.sha256`, `.<id>.reuse.<mode>`); unpack removes
them. Each release also installs its own dependencies in setup, so a
rollback to an open release finds them already in place; one kept only as
a backup gets them from setup again.

## Screens

| Key | Deployables | Operation | Releases |
| --- | --- | --- | --- |
| `↑` `↓` | select | scroll the log | select |
| `c` | connect, or disconnect when connected | | |
| `⏎` | deploy (asks first) | | roll back to the selected release (asks first) |
| `D` | | | the release's details and every file in it, read-only; a backup is read without unpacking it |
| `r` | open releases | open releases | refresh |
| `s` | refresh the connected ones | | |
| `x` | clean up the unfinished files the server reported (asks first) | | |
| `:` | a command for the selected service: `:logs`, `:restart`, `:setup`, `:stop`, `:start` | | |
| `G` | | follow the log again | |
| `esc` | | back | back |
| `tab`, `1` `2` `3` | switch screens | | |
| `q` | quit (asks while something runs) | | |

Nothing connects when shipdeck starts. `c` connects the selected deployable
and shows what is live; deploying, opening releases and rolling back connect
on their own. Disconnecting closes the session, so every deployable on that
`user@host:port` shows as not connected.

When a release fails after the switch, the rollback question takes `y` (roll
back now) or `n` (keep it live).

`:` opens a command line at the bottom, as in vim. Tab completes a command
(again for the next match), Enter runs it, Esc cancels, and an unknown one
says which there are:

| Command | Does |
| --- | --- |
| `:logs` | The service's last 200 log lines, then each new one live, over the whole screen: `journalctl -u <service> -f` (with `sudo -n` unless root), or `pm2 logs <app> --raw`. `↑` `↓` scroll, `G` follows again, `q` or `esc` closes it and ends the command on the server. |
| `:restart` | Restarts the service, as a deploy's restart step does. Asks first. |
| `:stop` | Stops it until it is started again, a deploy restarts it or the server reboots. Asks first. |
| `:start` | Starts it; a running one is left as it is. |
| `:setup` | Guided one-time server setup. Choose full sudo or only the commands shipdeck runs, then enter the sudo password. It never widens access without that choice. |

While a deploy or rollback of that deployable runs, only `:logs` works. A
command the server's sudo rule does not allow says so, with the
`server-setup.sh` command that renews the rule.

