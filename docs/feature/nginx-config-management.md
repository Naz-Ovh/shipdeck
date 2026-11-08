# Structural Nginx configuration management

Status: agreed plan, 2026-11-07. Implementation has not started.

## Problem and decision

Shipdeck must set up a fresh VPS and manage Nginx without asking the owner to
edit configuration files. It must also read and edit system configuration
structurally, including files outside its app configuration directory.

Build and maintain our own Rust configuration library. Use Nginx's actual
parser implementation as the grammar reference. Do not add Rowan,
nginx-lint-parser, or Crossplane as runtime dependencies. The parser core should
use the Rust standard library only. Any later dependency needs approval.

The owner chose ownership and fewer dependencies. This is not a claim that our
implementation will be faster or more correct than existing libraries.

## Scope and boundaries

The library reads Nginx configuration into structures, edits those structures,
and writes configuration back. It covers global settings, events, HTTP, stream,
mail, and site blocks through generic directive and block nodes.

Structural support does not mean understanding every module. Interpret settings
that Shipdeck manages through explicit typed rules. Preserve unknown directives
and their arguments. Do not infer their meaning or silently rewrite them.

Nginx remains the authority for module availability, directive context,
inheritance, and full configuration validation. The library does not replace
`nginx -t`. Module-defined languages such as embedded Lua require explicit
support; unsupported syntax must produce an error and leave files untouched.

This plan defines the configuration foundation. It does not implement mail
delivery, FTP, certificate issuance, cluster scheduling, or a new web server.
The server-agent work remains separate. Root access remains subject to the
existing p3d1 decision.

## App-facing nginx settings

Owner direction, 2026-11-09: when nginx is implemented, a project gets a
dedicated nginx settings block. It is not mixed into `service` or `static`.
Everything that only makes sense for nginx belongs here. At least:

- response headers;
- `location` blocks;
- an override for the served path or root;
- raw nginx configuration as a string block, as the escape hatch.

A rough shape, not an agreed field list:

```yaml
dir: /var/www/admin
static: {root: dist}

nginx:
  headers:
    X-Frame-Options: DENY
  locations:
    /assets/: {expires: 30d}
  root: dist/public        # override, when it differs from static.root
  raw: |
    # arbitrary nginx directives, validated before use
```

Still undecided:

- the block name (`nginx`, `serve`, `web`) and whether it sits on the deploy
  or on each app;
- which settings get typed fields and which appear only in the raw block;
- how a typed `location` and raw text combine, and which wins;
- what a path override means next to `static.root`; one must win, never both
  silently;
- domains, TLS and certificates, which stay with the domain phase.

Rules that do not change:

- shipdeck owns its generated site file. Raw text is validated with `nginx -t`
  as part of the whole candidate before activation.
- A failed validation leaves the previous file and the running configuration
  untouched.
- Raw text never overwrites unrelated nginx files.
- A deployment records the routing settings it used, so rollback can restore
  them or refuse to pretend it did.

This section records direction only. The field shape is not agreed until the
owner confirms it and implementation is scheduled.

## Data model

Define public types and signatures before implementing non-trivial modules.

- A document owns its original source bytes and an indexed node collection.
- Tokens retain source ranges for whitespace, comments, delimiters, and values.
- Directive nodes retain their name, ordered arguments, and optional block.
- Store raw spelling separately from decoded argument values.
- Node identifiers and source ranges locate exact edits without guessing.
- A configuration graph records files, include occurrences, and their contexts.
- Errors carry the file, byte range, line, column, and concrete cause.

Use byte ranges so unrelated content, including non-UTF-8 path bytes, is not
silently changed. Validate conversions before displaying text or creating typed
settings. Prefer borrowed slices and checked offsets. Do not add unsafe code,
shared ownership, or locking just to simplify implementation.

## Parsing and writing

Follow a recorded Nginx source revision for token boundaries, quotes, escapes,
comments, variables, semicolons, and braces. Keep a compatibility record for
each Nginx version tested. Do not treat a convenient simplified grammar as
equivalent to Nginx.

An unchanged document must write back byte-for-byte, including line endings,
comments, indentation, and final newline state. Edits replace exact source
ranges. New nodes use one documented formatting policy. Reject overlapping
edits and stale node references. Reparse the result before further edits.

Do not use regular expressions to identify configuration structure. Do not
normalize untouched text. Argument encoding must prevent values from becoming
extra directives or blocks and must preserve literal versus variable meaning.

## Complete configuration discovery

Discover the executable, service startup arguments, configuration path, prefix,
and loaded modules. Respect explicit startup paths instead of assuming a Linux
distribution's default layout.

Resolve relative includes using Nginx's configuration-prefix rules. Expand
wildcards with ordering and unmatched-pattern behavior checked against Nginx.
Retain each include occurrence and its parent context, even when a file is
included more than once. Detect cycles without rejecting legitimate repeated
includes. Enforce documented depth, file-count, and total-byte limits with
explicit errors. Track symlinks and file identity before modifying files.

Disk configuration and running configuration are different states. Show their
source and status separately. `nginx -T` validates and dumps disk configuration;
it must not be described as proof of the configuration currently serving traffic.

## Management and activation

The agent exposes parsed settings, source locations, raw files, and proposed
diffs to CLI and TUI. Editing a supported field targets a specific node. Unknown
settings remain available as source and preserved structures.

Prepare candidate files and check that source fingerprints have not changed.
Serialize writers through one configuration transaction. Validate the complete
candidate graph with the installed Nginx binary, preserving include and prefix
behavior. A site-only validation is insufficient.

Keep a recoverable snapshot before changing active files. Multi-file updates
are not inherently atomic: define activation and crash recovery explicitly.
Only request reload after all intended files are installed and validated.
Confirm reload results and probe affected routes. A successful signal command
alone is not proof of successful activation. On failure, restore the previous
files, validate, reload, and report any recovery failure.

Saved deployment settings must identify their routing configuration version so
app rollback can restore routing when needed. Never overwrite an external edit
silently. Preserve unrelated system settings during app deployment.

## Implementation steps and acceptance

This supports the existing Nginx/domain work. It does not mark a tracker step
built or change its priority. Assign implementation substeps when scheduling it.

1. Record the reference source revision and define types, errors, and limits.
2. Build the lossless lexer, parser, and unchanged-source writer.
3. Add structural edits and correct argument encoding.
4. Add installation discovery, include resolution, and configuration graphs.
5. Add typed views and edits for settings Shipdeck manages.
6. Add agent transactions, inspection, activation, and recovery.
7. Connect the existing RPC, CLI, and TUI paths when implementing management.

Required verification:

- Byte-for-byte parse/write tests across representative system configurations.
- Quotes, escapes, variables, regex arguments, comments, CRLF, non-ASCII bytes,
  missing delimiters, empty blocks, long tokens, and truncated input.
- Include ordering, prefixes, wildcard behavior, repeated includes, cycles,
  symlinks, missing files, and resource limits.
- Add, update, and remove operations preserve unrelated bytes and file ownership.
- Differential tests against real Nginx versions for syntax handled by the core
  parser. Separate syntax failures from environment or module failures.
- Invalid candidates, external edits, reload failures, interrupted writes, and
  rollback failures through SSH in disposable Podman servers.
- Formatting, relevant tests, and strict project Clippy checks.

Measure parsing time, allocations, and memory on recorded inputs before making
efficiency claims. Do not trade compatibility or safe bounds for speed.

## References

- [Nginx parser source](https://github.com/nginx/nginx/blob/master/src/core/ngx_conf_file.c).
- [Nginx configuration types](https://github.com/nginx/nginx/blob/master/src/core/ngx_conf_file.h).
- [Nginx license](https://github.com/nginx/nginx/blob/master/LICENSE).
- [Earlier control research](./nginx-control-research.md).

If adapting upstream source, preserve applicable license and copyright notices.
