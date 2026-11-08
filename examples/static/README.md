# Static site, no process

`shipdeck.yaml` packs `dist`, switches `current` to it, and runs no service.
nginx serves the folder once, and every deploy changes the files behind it:

```nginx
root /var/www/static/current/dist;
```

Change the SSH settings, then from the repository root:

```sh
shipdeck deploy examples static --key ~/.ssh/id_ed25519
```

`dir` holds `current`, `releases/` and `backups/`. `static.root` must be in the
packed files; the pack step stops when it is not. A health URL checks
the site through nginx. See the [feature plan](../../docs/feature/static-site-mode.md).
