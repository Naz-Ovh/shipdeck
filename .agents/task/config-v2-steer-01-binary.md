# Steer 1: `binary` takes `{ path, to }`

Created: 2026-11-11. A change to the task in
[config-v2-rewrite.md](config-v2-rewrite.md). The owner decided it after you
started. It wins over anything you built before it.

## The change

`apps.<app>.binary` now takes the same two forms as a file entry:

```yaml
binary: target/release/api                          # lands at apps/api/api
binary: { path: target/release/api, to: bin/api }   # lands at apps/api/bin/api
```

The design page and both example files are already updated. Read the
`binary` lines again in:

- [config-v2.md](../../docs/feature/config-v2.md): "Where files land", the
  short `binary` section under it, and the process `start` default.
- [reference.yaml](../../docs/feature/config-v2/reference.yaml) and
  [standalone.yaml](../../docs/feature/config-v2/standalone.yaml).

## The rules

1. **One type.** `binary` uses the same file-entry type as `files`: a plain
   path, or `{ path, to }`. Do not add a second type or a `from` key.
2. **Where it lands.** Always in the app folder: `apps/<app>/<file name>`,
   or `apps/<app>/<to>` when `to` is given. Mode `0755` in both forms.
3. **`to` is optional.** Unlike a file outside `source`, a binary never needs
   `to`, because its file name is its place.
4. **`path`** follows "Paths on your PC", like any file. It must be one
   file. A folder is an error that names the path.
5. **`to`** follows the existing rules for paths in the release: no leading
   `/`, no `..`, nothing into `.shipdeck/` or a shared folder. A binary and a
   file that land at the same path are the "one path, one source" error.
6. **Default `start`** is the binary's place in the app folder:
   `./<file name>`, or `./<to>` when `to` is given.
7. **Static apps** still refuse `binary`, in both forms.

## Tests to add

In the sibling `_tests.rs` files of the modules you change:

- Both forms parse. The map form without `to` behaves like the plain form.
- A binary with `to` lands at `apps/<app>/<to>` with mode `0755`, in the
  packed release and in `shipdeck tree`.
- The default `start` uses `to` when given.
- Refused: a folder as `path`; `to` with `/` or `..`; `to` into `.shipdeck/`;
  a binary and a file at the same path; `binary` on a static app.

## Report

Add a line for this steer in the decisions part of
`.agents/task/config-v2-rewrite-report.md`. If your work already used
`binary` as a plain path only, say what you changed.
