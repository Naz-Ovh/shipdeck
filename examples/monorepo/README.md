# Two apps, one release

This example runs an API with systemd and a web app with PM2. Change the SSH
settings before deploying. The server needs Node.js, PM2, zstd, and curl or wget.
The SSH user must be root, or have full password-free sudo.

From the repository root, run:

```sh
shipdeck deploy -C examples/monorepo production --key-file ~/.ssh/id_ed25519
shipdeck releases -C examples/monorepo production --key-file ~/.ssh/id_ed25519
```

Both apps share one release and roll back together. Their uploads folders stay
under `/opt/product/shared/apps/`. See [Apps in config format 2](../../docs/feature/config-v2.md#apps).
