# Agent access and version policy

Status: implemented and verified in Podman, 2026-11-08. The release version is
not selected. This change breaks the old installation and version contract, so
it needs a major release under the rules below.

## Installation and access

Shipdeck connects from a local CLI or CI through SSH using an authorized
account. Preparation happens during the first connection. There is no separate
setup command. The agent needs elevated access for protected files, systemd,
and system dependencies.

The account must be root or have password-free sudo (`NOPASSWD`). Shipdeck
uses `sudo -n` and never asks for a sudo password during preparation. The rule
must allow the agent installer and launch, so a rule limited to the service
commands is not enough. When the account cannot use sudo, the connection stops
with a clear access error before anything changes.

Agents live at `/var/lib/shipdeck/agent/v<version>/agent`. Root owns the executable
and every parent directory. The directories are not writable by ordinary
users, and the executable is `0755`. Shipdeck starts it through `sudo` with the
full path. There is no `PATH` entry, shared executable, or `current` symlink.

GitHub is the only release source. The fixed repository and the versioned
release archive and `.sha256` asset are used.

## Installation integrity

The installer runs as root inside one install lock. It uses `flock` when it is
available and an atomic directory lock with stale-owner detection otherwise.
It downloads into a private staging directory, checks the archive checksum,
paths, entry types, and the printed version, then publishes a complete version
directory with one rename.

The install never executes a user-writable path as root. A development binary
from `SHIPDECK_AGENT_BINARY` is first uploaded to the user's private folder.
Root copies it once into staging, checks the copied bytes, and runs only the
root-owned copy.

A concurrent connection waits for the lock and then reuses the published
installation. An interrupted or failed install leaves no valid version
directory. Older version directories are removed after a successful publish.

## Compatibility and updates

Shipdeck checks the installed agent version on every connection, before
operational requests. It compares the agent's major version with the connecting
Shipdeck's major version:

| Comparison | Behavior |
| --- | --- |
| Same major | Use the newest installed same-major agent. Different minor and patch versions are allowed; exact equality is not required. |
| Agent major is greater | Refuse the connection. Show both versions and the required agent major. Do not downgrade the agent or use another installation to bypass the refusal. |
| Shipdeck major is greater | Remove older agent installations, install the release for the connecting Shipdeck, start it, and complete the handshake. |
| Agent missing | Install the release for the connecting Shipdeck, start it, and complete the handshake. |

A newer agent major blocks the connection even when a same-major installation
is also present. Versioned directories are an installation layout, not a way
around these rules. A failed install or handshake ends the connection; no
operation runs through that agent.

## Execution identity

The agent runs as root. App commands and PM2 do not inherit that identity: the
request carries the service user, and the agent starts PM2 through `runuser`
with that user's home and login `PATH`. Systemd and system package work may run
as root.

Deployment files are given back to the SSH user after creation, so existing
app and service users keep their files. Project, release, backup, and shared
paths remain under the project root. The path, archive, symlink, and mode
checks stay in force under the elevated agent.

## Breaking changes

Changes to the protocol, execution identity, or file-handling contracts that
break existing behavior require a major-version bump. Minor and patch releases
must remain compatible within their major. Do not change deployment contracts
inside a compatible minor or patch release.

This implementation changes the install location, the privilege boundary,
the execution identity, and the agent protocol. It therefore requires a major
release. The owner must choose and publish that version; this tree does not
select one.

## Verified

Podman tests covered first install, reconnection, same-major `0.6.9`,
newer-major refusal, older-directory cleanup, simultaneous installs, an
interrupted download, a bad checksum, a wrong binary version, a failed
handshake, root ownership, failed replacement by ordinary users, development
upload, missing sudo, a PM2 deploy, a systemd deploy, a systemd setup command
running as `shipdeploy`, and non-interactive CLI use.

The CLI-major-greater rule and its cleanup decision are covered by unit tests.
The current tree stays at `0.7.1`, so a real major upgrade was not published or
run.
