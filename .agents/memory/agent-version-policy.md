# Agent compatibility and privileged installation

Owner decision, 2026-11-08. Source: user-stated.

Follow [the agreed policy](../../docs/feature/agent-version-policy.md).
Agents use root-owned `/opt/shipdeck/agent/v<version>/agent` paths and are
launched by full path. GitHub is the only release source. Preparation happens
on connection, without a separate setup command.

Allow minor and patch differences within the same major. Refuse when the
agent's major exceeds Shipdeck's; report the required agent version change.
When Shipdeck's major exceeds the agent's, clear older installations and install
the newer major. Breaking protocol, execution, or file-handling contract changes
require a major release. Do not restore exact-version matching as the design.

These are saved requirements, not implemented behavior.
