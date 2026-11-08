# Shipdeck

Shipdeck builds your app on your computer, sends it to a Linux server over SSH,
and runs it with **systemd or PM2**. Use its terminal UI or commands for scripts
and CI. Each deployment gets its own release folder. Inspect releases, follow
logs, control services, and roll back to an earlier release.

It fits small servers that should run your app without also building it.
Rust, Go, Node.js, Python, and other stacks work through your build and setup
commands. A monorepo can deploy several apps together as one release.

## Get started

1. **Install** on Linux or macOS:

   ```sh
   curl -LsSf https://shipdeck.naz.ovh/install.sh | sh
   shipdeck --version
   ```

   For a pinned release, append `-s -- -V=v0.8.1` to `sh`.
   Windows currently needs a source build and Git for Windows.

2. **Create a deployment** from your repository root:

   ```sh
   shipdeck init ./scripts/deploy my-api --preset rust --manager systemd --host prod
   ```

   `prod` is an SSH alias. Set its user and `IdentityFile` in your SSH config,
   or use a server address and pass `--key` when connecting. Edit the new
   `scripts/deploy/my-api/deploy.yaml`: check the project path, build command,
   packed files, server folder, start command, and health check. Build a Linux
   binary for the server's CPU and libc. Keep secrets out of git.

3. **Deploy**:

   ```sh
   shipdeck ./scripts/deploy
   ```

   Select the deployment and press Enter. `c` connects, `r` opens releases,
   `D` inspects a release, and `:logs` follows logs. Select an older release and
   press Enter to roll back. `q` quits.

For scripts, run `shipdeck deploy ./scripts/deploy my-api`. It asks no questions;
provide login details and a failure policy. Run `shipdeck deploy --help`
for those flags. Review a new host key before trusting it.

## What happens on a deploy

`preflight → build → pack → upload → unpack → setup → backup → switch → restart → health → done`

- **Your computer** builds and packs only the files you list. Unchanged files
  can be reused after server-side SHA-256 checks.
- **The server** prepares and verifies a new release beside the live one.
  Setup installs runtime dependencies there. A single rename switches the
  `current` symlink, then services restart and health checks run.
- **Rollback** returns to a saved release and its service settings. Compressed
  backups run setup again. Shared files survive release changes. Database
  migrations and your custom commands need their own recovery plan.

Shipdeck prepares and runs a matching **shipdeck-agent** over SSH. The first
connection needs an account with root or password-free sudo. The agent installs
as root under `/opt/shipdeck/agent/v<version>/agent`, handles files, backups,
service operations, project locks, and events, and starts through the full path
with sudo. It opens no network port and exits on disconnect. Published builds
download the checked release from GitHub. If preparation or the handshake
fails, the connection stops instead of using an incompatible agent. App and PM2
commands still run as the configured service user.

Deployments run while connected. If the connection drops, reconnect and run:

```sh
shipdeck work ./scripts/deploy my-api
shipdeck events ./scripts/deploy my-api --after 0
```

Interrupted files stay for inspection. `work --cleanup` explicitly approves their
removal after a fresh server check. A lost connection during activation is an
uncertain result: inspect the live release and services before retrying.

## Read only what you need

| Need | Start here |
| --- | --- |
| First deployment, with a complete config | [Quickstart](https://shipdeck.naz.ovh/docs/getting-started/quickstart) |
| Config, flags, pipeline, and examples | [Configuration reference](docs/feature/deployment-config.md) |
| Setup, agent behavior, server layout, and controls | [Operations reference](docs/feature/deployment-operations.md) |
| Several apps in one release | [Monorepos](docs/feature/monorepo-deploy.md) |
| Work on Shipdeck or publish a release | [Development](docs/code/development.md) |

For a coding agent using Shipdeck, the **installed manual** is the shortest source:

```sh
shipdeck man index
shipdeck man configs
shipdeck man details agents cli.deploy service.start
```

Online: [llms.txt](https://shipdeck.naz.ovh/llms.txt) gives the overview and topic
links. Fetch a topic only when needed; [llms-full.txt](https://shipdeck.naz.ovh/llms-full.txt)
contains every page. Contributors start with [AGENTS.md](AGENTS.md), then the
[current handoff](.agents/handoff.md).
