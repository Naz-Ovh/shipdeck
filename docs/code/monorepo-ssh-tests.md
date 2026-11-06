# Monorepo SSH tests

Run `cargo build -p shipdeck`, then:

```sh
python3 tests/monorepo_ssh.py \
  --container shipdeck-monorepo-ssh-test \
  --port 38733 --key /tmp/shipdeck-ssh-test/id_ed25519
```

The test needs an existing disposable Podman container with systemd, SSH on the
published port, Node.js, PM2, zstd, tar, sha256sum, and curl or wget. The `deploy`
user must accept the supplied key and have passwordless sudo. No package is
installed by the script.

The script creates a unique deployment under `/tmp` inside the container.
Local sources and command logs are kept in a printed temporary folder. Test
processes and releases remain in the disposable container for inspection.
Remove the container when inspection is finished.

Rootless Podman cannot apply the built-in systemd unit's mount namespaces.
The test uses a custom service template without those settings. It still tests
systemd installation, restart, health, stopping, disabling, and manager changes.
Production service hardening is unchanged.

The suite checks inline and external app configs, mixed managers, global and
app hooks, environment files, unchanged detection, shared data, retention,
compressed backup rollback with local settings that differ, setup failure,
failed restore setup, partial activation recovery, done-hook warnings, custom
PM2 ecosystem backups, automatic recovery after app and manager changes, and RPC status, controls,
release inspection with damaged live metadata, and combined logs.
