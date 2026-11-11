# Development and release

Start with the [overview](../../README.md). This page covers the repository.

## Code map

| Path | Responsibility |
| --- | --- |
| `apps/cli/` | CLI, engine child process, console output, built-in manual. |
| `crates/engine/` | Config, release plan, local builds, SSH, one deploy pipeline, `tree`. |
| `crates/agent/` | Linux server helper: files, services, locks, backups, events. |
| `crates/protocol/` | Typed messages shared by engine, clients, and agent. |
| `crates/tui/` | Terminal screens, dialogs, and input. |
| `docs/website/content/docs/` | Published user docs; also exported as Markdown. |

The engine reports through the protocol. CLI and TUI show those reports.
The agent speaks JSON-RPC on stdin/stdout over SSH; diagnostics go to stderr.
See [agent behavior](../feature/agent.md) for its safety boundaries.

## Build and check

Run `just` for all recipes:

```sh
just build                    # whole workspace, including the agent
just build release            # optimized workspace
just run --help
just test
just verify                   # fmt, check, strict Clippy, workspace tests
just build-doc                # website, with installed pnpm
python3 tests/release_artifacts_test.py
```

A CLI-only build does not build the agent. For development connections, point
`SHIPDECK_AGENT_BINARY` at your matching Linux agent binary. Published builds
download the release for the connecting CLI; an installed agent with the same
major version is reused even when its minor or patch differs. Do not treat the
development path as a download fallback.

The agent is installed under `/opt/shipdeck/agent/v<version>/agent` by root.
The SSH account must be root or have password-free sudo. A development binary
is uploaded to a private user folder, copied into root staging, checked, and run
only from the root-owned copy.

To install both programs from this checkout on Linux:

```sh
just install
just install-agent
shipdeck-agent --version
```

Then connect using the SSH host, user, and key in your deploy file:

```sh
SHIPDECK_AGENT_BINARY="$(command -v shipdeck-agent)" shipdeck server-info production
```

Replace the deploy name with your own. Shipdeck uploads the agent,
checks its checksum and version, and starts it over SSH. You do not need to log
in and install it by hand. The binary must match the server's Linux CPU and libc.
Its version must have your Shipdeck's major version. For another server target,
build the matching Linux agent and set `SHIPDECK_AGENT_BINARY` to that file.

Config types generate the editor schema. Run `just schema` after config changes.
Run `just protocol` after protocol changes. The manual lives in
`apps/cli/manual/`; keep it consistent with behavior.
`SHIPDECK_DUMP=/tmp/frames cargo test -p shipdeck-tui` exports screen captures.

Local Lefthook hooks run the version check before commits and Rust checks before
pushes. They do not replace release verification.

The version tool is an independent Cargo project in `scripts/version_bump/`, outside
the application workspace. It has no dependencies. From the repository root,
run `just version NEW_VERSION` or:

```sh
cargo run --quiet --manifest-path scripts/version_bump/Cargo.toml -- NEW_VERSION
```

## Publish a release

1. Inspect the changes since the last published tag and the relevant reviews.
   Run `just verify`, build the docs, and check generated schemas.
2. If the workspace version has not already been raised, run
   `just version NEW_VERSION`. It updates manifests, lockfile, examples, README
   URLs, and schema URLs. Cargo builds only the version tool. Update website
   examples and changelog notes too. Regenerate schemas separately after type
   changes.
3. Run `dist plan` and `dist generate --check`. After changing dist config,
   run `dist generate` and inspect its changes. The pinned tool version is in
   [dist-workspace.toml](../../dist-workspace.toml).
4. Commit with a conventional title, for example
   `chore(release): prepare v0.7.1`. Tag that verified commit and push:

   ```sh
   git tag v0.7.1
   git push origin main v0.7.1
   ```

The [release workflow](../../.github/workflows/release.yml) builds six CLI
archives (Linux x86_64/arm64, GNU/musl; macOS x86_64/arm64) and **four Linux
agent archives**. The agent is archive-only, with no user installer.

Before publication, the [artifact gate](../../.github/workflows/glibc-floor.yml)
requires all four agent archives and their SHA-256 files, checks each checksum,
executable member, and ELF CPU, then checks the GNU binaries' glibc floors.
CLI GNU needs at most glibc 2.28; agent GNU needs at most 2.31. Older Linux uses
musl. A failure blocks publication.

The workflow creates the GitHub Release after these checks. Do not create it
by hand first. Stable publication then calls the docs deployment workflow.
Watch the complete workflow, verify the public assets and binary versions,
and confirm the live docs before calling the release complete.

32-bit ARM and prebuilt Windows binaries are not shipped. Disk-full behavior
has not been verified. Upload speed has no measured gain over direct SSH/SFTP.
