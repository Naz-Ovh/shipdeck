# Monorepo deploy config

## Goal

Deploy several apps from one repository as one deploy. The deploy has one
global name and named apps. Shipdeck bundles everything at once in one deploy
directory. Global steps and app-specific steps can both be configured.

## Agreed config design

- A deploy has one main `deploy.yaml`.
- All apps share one release. If an app fails after the switch, rollback
  restores the previous release for all apps. External changes, such as
  database migrations, are not automatically undone.
- Apps can be configured inline in that file or in separate files. Separate
  app files are recommended when the config grows.
- A global `process_manager` sets the default for apps. Each app can customize
  its process manager. An app's `process_manager` replaces the whole global
  section; it does not inherit its fields.
- `systemd` is the default manager. `pm2` is also supported today. `oxmgr` is
  planned, not supported yet.
- Nginx will have its own config section. Nginx is the planned web server for
  this feature. Its fields and placement are not settled yet.
- Pipeline options are available globally and can be overridden in an app's
  config. The exact fields, override rules and execution order are not settled.
- Nginx support is the next plan. The wider goal is a complete deploy, from
  setup to a live site, with Shipdeck controlling Nginx programmatically.

The file names and YAML below show the intended shape. They are not a finished
schema, and Shipdeck does not support them yet.

```text
deploy/
├── deploy.yaml
└── apps/
    ├── api.yaml
    └── web.yaml
```

```yaml
# deploy.yaml
name: my-product

process_manager:
  name: systemd

apps:
  api: apps/api.yaml
  web:
    start: node server.js
    process_manager:
      name: pm2
```

```yaml
# apps/api.yaml
start: ./api
# Uses the global process manager by default.
```

## Still to decide

- The exact YAML keys, file reference syntax and path rules.
- How pipeline overrides work and in what order commands run, including before
  and after commands.
- How the global deploy name and app names form process names.
- The release layout and the details of switching and restarting multiple apps.
- Nginx settings, HTTPS, and how Nginx changes are applied.
- How existing single-app configs remain compatible or move to the new layout.

## Tracker

This design has no tracker step yet. Implementation has not been scheduled.
