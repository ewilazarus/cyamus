# Proposal

## Why

Every Orca worktree that runs a docker compose stack exposes its services on `localhost:<some port>`. Those ports either collide between worktrees or are ephemeral and hard to find. cyamus knows which worktree belongs to which project and branch, so it can give every service a stable, readable URL such as `http://web.feat-x.myproj.localhost:1355` that works in the browser and from host processes. Docker compose is the main way services run, so Docker discovery is a first-class source, not an add-on.

## What Changes

- **Workspace registry.**
  - `cyamus workspace setup` records the workspace (project, branch, hostname label, worktree path) in `$XDG_STATE_HOME/cyamus/workspaces/`.
  - `cyamus workspace teardown` removes the record.
  - Records whose worktree no longer exists are garbage-collected.
  - This is the daemon's routing input.
- **Daemon.**
  - `cyamus daemon run | status | stop` is a single machine-wide HTTP reverse proxy, built into the same binary.
  - It listens on loopback only (`127.0.0.1` and `::1`), on port `1355` by default, overridable with `CYAMUS_DAEMON_PORT`.
  - `cyamus daemon install` is a one-time step using `sudo`. It installs a small **port-80 relay**: a boot-time service listening on `127.0.0.1:80` and `[::1]:80` only, forwarding TCP to the daemon port. URLs then need no port (`http://web.feat-x.myproj.localhost/`). The relay runs unprivileged (as `nobody` under launchd on macOS, as a systemd `DynamicUser` on Linux) from a root-owned copy of the binary. `cyamus daemon uninstall` removes it. Without it, everything keeps working on `:1355`.
  - Setup starts it on demand in the background and restarts it when the running daemon is a different cyamus version.
  - It runs until `cyamus daemon stop`.
- **Routing.**
  - `http://<service>.<branch-label>.<project>.localhost[:<port>]` is forwarded to the service's published host port.
  - The original `Host` header is preserved, `X-Forwarded-*` headers are added, and WebSocket upgrades pass through (HMR).
  - Unknown hosts get a 404 that lists the current routes, and `http://cyamus.localhost:<port>` shows the route table.
- **Docker discovery.**
  - A container joins a workspace automatically when its compose `working_dir` is inside a registered worktree, so the compose file needs no per-branch values.
  - `cyamus.*` labels override the service name and target port, opt a container out (`cyamus.enable=false`), or attach a container that wasn't started by compose to a workspace.
  - The daemon follows Docker's event stream, so routes appear and disappear as containers start and stop.
- **Hook environment.**
  - `COMPOSE_PROJECT_NAME=<project>-<branch-label>` is exported unless already set, so stacks from different worktrees don't collide.
  - `CYAMUS_DOMAIN` (`<branch-label>.<project>.localhost`), `CYAMUS_PROXY_PORT` and `CYAMUS_URL_SUFFIX` are exported. `CYAMUS_URL_SUFFIX` is empty when port 80 reaches the daemon, and `:<port>` otherwise. Hooks and compose interpolation can then build service URLs as `http://web.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}`.
- **Documentation.**
  - The README covers the "compose file as a copied asset" pattern, including `compose.override.yaml` for repos that already track a compose file.
  - It explains publishing ephemeral ports, `cyamus.*` labels, and container-to-container calls (use compose service names, not `*.localhost`).

Out of scope, and captured in the design as future work:
- HTTPS (local CA, per-host certificates, trust installation) and port 443.
- `[[service]]` port allocation for dev servers run on the host.
- `cyamus compose` for project-shared services.
- Templated compose project names.

## Capabilities

### New Capabilities
- `workspace-registry`: the on-disk record of active workspaces: location, contents, writing on setup, removal on teardown, garbage collection, and hostname label derivation and conflicts.
- `daemon`: the background proxy process: listen address and port, on-demand start from the CLI, single-instance guarantee, version handover, the `daemon run/status/stop` commands, the optional port-80 relay (`daemon install/uninstall`), and the relay's forwarding (`daemon relay`).
- `proxy-routing`: the hostname scheme and request forwarding: resolving a Host to a target, header handling, WebSocket passthrough, unknown-host and backend-down responses, and the `cyamus.localhost` route page.
- `docker-discovery`: how containers become routes: workspace inference from compose `working_dir`, the `cyamus.*` label contract, service name and port selection, published-port-only targeting, and live updates.

### Modified Capabilities
- `workspace-lifecycle`:
  - Setup registers the workspace and ensures the daemon is running before hooks run.
  - Teardown unregisters the workspace after its hooks.
  - The hook environment gains `COMPOSE_PROJECT_NAME`, `CYAMUS_DOMAIN`, `CYAMUS_PROXY_PORT` and `CYAMUS_URL_SUFFIX`.

## Impact

- **New crates:**
  - `cyamus-registry` is synchronous and used by both the CLI and the daemon.
  - `cyamus-daemon` is async (tokio, hyper, bollard), linked into the same `cyamus` binary.
  - `cyamus-core` gains no async or network dependency.
- **Dependencies:** tokio, hyper, hyper-util, http-body-util, bollard (Unix socket only, without TLS features), and a file-lock crate. All are pure Rust, so the musl release builds are unaffected. axum-reverse-proxy is the fallback only if plain hyper proves insufficient.
- **New state:** `$XDG_STATE_HOME/cyamus/` holds `workspaces/`, `registry.lock`, `daemon.pid`, `daemon.lock` and `daemon.log`.
- **System configuration, only when the user runs `cyamus daemon install`:**
  - Both OSes: `/usr/local/libexec/cyamus-relay`.
  - macOS: `/Library/LaunchDaemons/dev.cyamus.relay.plist`.
  - Linux: `/etc/systemd/system/cyamus-relay.service`.
  - Setup never touches system configuration.
- **CLI:** a new `daemon` subcommand group. `workspace setup` now starts a background process, which can be disabled with `CYAMUS_DAEMON=off`.
- **README:** new "Daemon and routing" and "Docker" sections.
