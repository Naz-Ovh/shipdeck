# Programmatic Nginx control: research

Checked on 2026-11-07 against official Nginx and Certbot documentation.
This is a research recommendation, not an agreed schema or implementation plan.
No application code or server configuration was changed for this research.
The separate server-agent work remains with the other LLM.

## Recommendation for Shipdeck

Have the Shipdeck agent manage a clearly owned set of Nginx configuration files.
Generate those files from typed routing settings. Validate the complete candidate
configuration, apply it, reload gracefully, and check the actual routes.

Use the new Control API when the installed binary supports it and a protected
local socket is enabled. Keep command-line control for other installations.
This is a proposed compatibility design; neither backend has been implemented
or tested in this research session.

The existing project design already calls for native Nginx, project domains,
HTTPS, and agent-side control. Root access is still an open decision, p3d1.
See the [project tracker](../project-progress.html#p3d1) and
[agreed design](../visual/Plan/spec.html).

## What Nginx actually supports

| Interface | Verified capabilities | Implication for Shipdeck |
| --- | --- | --- |
| Command line | Test configuration, dump on-disk configuration, reload, stop, quit, and reopen logs. | Compatible control path without a new Nginx HTTP listener. |
| Open Source Control API | Read loaded configuration, inspect workers/version, and request a full reload with a result. | Better feedback and a way to compare running settings with files. |
| Nginx Plus HTTP API | Runtime upstream changes, status information, and key-value management. | Optional commercial capability; unnecessary for normal domain routing. |

The command-line switches include `-t` for validation, `-T` for validation plus
configuration output, `-V` for build options, and `-s reload` for graceful reload.
`-c` selects the configuration file; `-p` sets the path prefix. Testing and
controlling a custom instance must use its actual configuration and prefix.
Source: [Nginx command-line reference](https://nginx.org/en/docs/switches.html).

A successful reload starts new workers while older workers finish current
requests. If the master cannot apply a candidate, it keeps the old settings.
This protects the running instance, but does not repair bad files on disk.
Source: [Nginx reload behavior](https://nginx.org/en/docs/beginners_guide.html).

Sending a reload signal alone does not prove that the new settings became live.
A later reload failure is possible. Shipdeck must check the result and routes.
Source: [Nginx maintainer explanation](https://mailman.nginx.org/pipermail/nginx/2013-August/040193.html).

## The new Open Source Control API

Open Source 1.31.5 introduced the Control API on 2026-09-02.
Source: [official release history](https://nginx.org/en/CHANGES).

The binary needs the `--with-control-api` build option. A version number alone
is insufficient. Source: [Nginx build options](https://nginx.org/en/docs/configure.html).

The master must be started with `-l`, specifying a Unix socket or a port.
A Unix socket is a local connection addressed through a filesystem path.
The API is disabled by default. Its documented routes include:

| Request | Result |
| --- | --- |
| `GET /1/nginx` | Nginx version and build name. |
| `GET /1/control/processes` | Worker process information. |
| `GET /1/control/config` | Loaded configuration files and contents. |
| `PATCH /1/control/config` | Full reload; success or failure with reload logs. |

These are control and inspection endpoints. They do not offer a general API for
adding a domain or writing a replacement server block. Shipdeck still needs to
create configuration files. Source:
[Control API specification](https://docs.nginx.com/nginx/admin-guide/yaml/nginx-control-api/1/nginx_control_api.yaml).

The API has no built-in authentication or authorization. Nginx recommends a
Unix socket with restricted permissions. It should remain behind the SSH agent,
not become a public management port. Enabling it changes master startup options;
it is not just a new `location` in a site file. Source:
[official Control API announcement](https://blog.nginx.org/blog/nginx-control-api-view-in-memory-configuration-and-reload-via-http-requests).

This Control API is distinct from the commercial `ngx_http_api_module` API.
The latter supports upstream and key-value changes without a full reload.
Source: [Nginx Plus API module](https://nginx.org/en/docs/http/ngx_http_api_module.html).

## Proposed configuration ownership

Keep one file per deployment, containing the app domains and routes for that
shared release. A monorepo could have `api.example.com` pointing to one app and
`example.com` pointing to another. A worker app can have no public route.

Own one clearly named include directory. Discover how the installed Nginx main
configuration includes site files; do not assume Debian's layout on every OS.
Leave unrelated site files untouched. Keep generated settings separate from
explicit user extensions. Never use text replacement over arbitrary Nginx files.

Keep routing generations and the last verified applied generation in server
state. Check for manual changes before replacing a managed file. Conflicts need
a clear report rather than silently overwriting the owner's edits.

The exact shipdeck.yaml fields remain undecided. This research does not settle
whether domains belong at deployment level, app level, or a separate route map.

## Proposed update and recovery sequence

1. Discover the binary, version, build options, service command, active config
   path, prefix, include layout, and available management interface.
2. Take a server-wide Nginx update lock. Save the previously applied generation.
3. Render a complete candidate containing all intended changes for this release.
4. Test the complete candidate with `nginx -t`, including the existing sites,
   required modules, certificate files, and the correct path prefix.
5. Make the already-tested files active with an atomic replacement on the same
   filesystem. For multiple app files, use one managed generation switch.
6. Reload using the Control API, or request a graceful command-line reload.
7. Check HTTP routing and, for HTTPS, the requested hostname and certificate.
   With the Control API, also inspect loaded managed configuration.
8. Record success only after verification. On failure, restore the previous
   managed files, validate, reload, and check again. Report recovery failure.

These transaction steps are a Shipdeck design proposal, not a transaction API
provided by Nginx. A lock only coordinates processes that honor it. Manual edits
and other tools require conflict detection and a fresh validation before apply.
A crash between file replacement and verification needs a recovery journal.

Restore Nginx routing together with the release's saved app settings when a
failed deployment changed domains, ports, or static roots. Certificate renewal
state should survive application rollback; do not restore an expired certificate
just because an old application release is selected.

A graceful Nginx reload does not make app restarts interruption-free. That would
also need overlapping app processes, separate ports/sockets, readiness checks,
and connection draining. Do not claim that the current deployment switch gives
that guarantee.

## Permissions

Proposed boundary: keep the agent as the SSH user, with a small privileged
operation for validated Nginx changes. Allow that operation to update only the
managed configuration area and perform fixed validation/reload actions.

Treat write access to included Nginx configuration as privileged. General raw
configuration can select files and modules; a writable include directory is not
a harmless way around root access. A privileged operation must validate paths,
values, and ownership rather than accept arbitrary shell text.

The final choice must fit p3d1 and the other LLM's agent design. No new sudoers
rules, root service, dependency, or listener is installed by this research.

## HTTPS and WebSockets

Nginx has an official ACME module for automatic certificate management.
It is separately installed and supports HTTP-01 and TLS-ALPN-01 challenges.
Its package/module availability must be checked for each supported distribution.
The documented certificate identifiers do not support wildcards.
Source: [official ACME module](https://nginx.org/en/docs/http/ngx_http_acme_module.html).

Certbot is another option. `certonly --webroot` obtains certificates without
letting the Nginx installer rewrite Shipdeck's managed configuration. Webroot
validation can use a running web server. Automated DNS plugins provide another
path, including wildcard certificates. Issuance and renewal still need their
own configuration and verification.
Source: [Certbot user guide](https://eff-certbot.readthedocs.io/en/stable/using.html).

Recommendation: evaluate the official ACME module first on the chosen OS package
set. If it is unavailable, evaluate Certbot webroot for ordinary domains and
DNS validation for wildcards. The certificate tool is not selected yet.

WebSocket proxying needs explicit Upgrade and Connection header handling.
Older Nginx versions also need explicit HTTP/1.1 proxy configuration.
Long-lived connections need suitable timeouts or application ping frames.
Source: [official WebSocket guide](https://nginx.org/en/docs/http/websocket.html).

## Checks needed before implementation is called ready

- Real distro-package versions, build flags, include paths, and service units.
- Both command-line and Control API paths, including API unavailable/disabled.
- Invalid syntax, missing certificates, port conflicts, reload failure, and
  recovery failure. A failed change must leave unrelated sites intact.
- Multiple apps changed together, removed routes, changed ports, static roots,
  app failure after a routing update, and rollback to saved routing settings.
- Two simultaneous updates, manual edits, and a crash during each apply phase.
- Domain and certificate checks, WebSockets, renewal, and log/status reporting.
- Root-owned configuration with an unprivileged agent and the chosen p3d1 model.

No runtime Nginx experiment was performed in this research session. Installation
and distribution compatibility are open verification work, not assumed facts.
