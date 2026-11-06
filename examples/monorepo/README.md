# Two apps, one release

This example runs an API with systemd and a web app with PM2. Change the SSH
settings before deploying. The server needs Node.js, PM2, zstd, and curl or wget.
Systemd deployment requires the permissions created by the server setup script.

From the repository root, run:

```sh
shipdeck deploy examples monorepo --key ~/.ssh/id_ed25519
shipdeck releases examples monorepo --key ~/.ssh/id_ed25519
```

Both apps share one release and roll back together. Their uploads folders stay
under `/opt/product/shared/apps/`. See the [feature guide](../../docs/feature/monorepo-deploy.md).
