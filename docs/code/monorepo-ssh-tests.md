# Monorepo SSH tests

Run `cargo build -p shipdeck`, then:

```sh
python3 tests/monorepo_ssh.py \
  --container shipdeck-monorepo-ssh-test \
  --port 38733 --key /tmp/shipdeck-ssh-test/id_ed25519
```

The name is kept for the docs that point at it; it now runs the config format 2 suite.

The test needs an existing disposable Podman container with systemd, SSH on the published
port, Node.js, PM2, zsh, python3, zstd, tar, sha256sum, and curl or wget. The `deploy` user
must accept the supplied key and have passwordless sudo. No package is installed by the
script.

The script creates a unique project under `/tmp` on this PC and a deployment under `/tmp`
inside the container. Local sources and command logs are kept in a printed temporary folder.
Test processes and releases remain in the disposable container for inspection. Remove the
container when inspection is finished.

Rootless Podman cannot apply the built-in systemd unit's mount namespaces. The test uses a
custom `process.systemd.template` without those settings. It still tests systemd
installation, restart, health, stopping, disabling, and manager changes. Production unit
hardening is unchanged.

The suite covers, in order:

- a deploy from a root file with a systemd API, a PM2 worker and a static site;
- release files and `to`, and `.shipdeck/` for shipdeck's own files;
- shared folder links, and data kept across releases;
- app order (name order) and release commands before app commands;
- an unchanged deploy (exit 6);
- a rollback from a backup, with `setup.server` run again;
- a failure before the switch (setup), which leaves the live release alone;
- an automatic rollback after a health failure;
- a rollback by hand to an open release;
- `keep_failed`;
- every runner, local and on the server;
- the preflight check for a missing runner;
- RPC status, release listing and reading `.shipdeck/release.json`.

A missing container, `sshd`, or one of the tools stops the suite with the message that names
it; it does not silently skip a scenario.
