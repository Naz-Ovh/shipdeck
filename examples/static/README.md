# Static site, no process

`deploy.yaml` ships `dist`, switches `current` to it, and runs no process. Your web server
serves the folder once, and every deploy changes the files behind it:

```nginx
root /var/www/static/current/apps/site/dist;
```

The path goes through `current` and the app folder (`apps/site`), so it is the same for
every release; a rollback only moves the `current` link.

Change the SSH settings in `deploy.yaml`, then from this folder:

```sh
shipdeck deploy production --key ~/.ssh/id_ed25519
```

`dir` holds `current`, `releases/` and `backups/`. `files: [dist]` must find `dist`; the
build step stops when it does not, and `shipdeck tree production` prints the same list
without connecting. `health.url` checks the site through your web server.

A static app takes no `binary`, no `shared`, no `steps.setup` and no `steps.backup.include`:
nothing runs on the server. To put it in a release beside a process app, see
`examples/monorepo`.
