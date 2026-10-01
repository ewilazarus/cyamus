# Design

## Context

- `port-core` set up a Cargo workspace in which `cyamus-core` is synchronous and must never depend on async, network or Docker crates. Its design reserved `cyamus-registry` and `cyamus-daemon` crates, linked into the same binary.
- `lifecycle::setup` and `lifecycle::teardown` in core drive the steps, and the CLI is a thin clap layer that passes in `Dirs` and a `Reporter`. `Dirs` already knows the XDG config and cache homes, and `XDG_STATE_HOME` was planned for this change.
- Findings from exploration on the target machine (macOS, OrbStack):
  - `*.localhost` resolves through the system resolver, Python, Node and curl, and resolves to `::1` first. The proxy must therefore listen on IPv6 loopback too.
  - An unprivileged process can bind `:80` only on wildcard addresses (`0.0.0.0` or `::`), not on loopback. A loopback-only listener therefore needs a high port.
  - The Docker endpoint is OrbStack's socket, selected through a Docker CLI context.
- The user's workflow: a `compose.yaml` kept in the project's `assets/` is copied into each worktree by `[[copy]]`, `on_setup` runs `docker compose up -d`, and `on_teardown` runs `docker compose down`. A single asset file serves every worktree, so it can only carry static labels.

## Goals / Non-Goals

**Goals:**
- Zero-config routing for compose stacks started inside a worktree. `cyamus.*` labels are only needed to rename a service, pick a port, or opt out.
- The proxy never makes setup fail. Routing is a convenience, and lifecycle hooks are the critical path.
- Routes follow reality, as workspaces and containers come and go, with no restart and no manual refresh.
- `cyamus-core` stays free of async code, and the release builds stay pure Rust.

**Non-Goals:**
- HTTPS and port 443, `[[service]]` port allocation for host processes, and `cyamus compose`. These are covered under Future work below.
- Running at login (launchd/systemd units). The daemon starts on demand from setup.
- Routing *between containers* through `*.localhost`. Inside a container, `localhost` is the container itself; compose service names remain the way containers reach each other.
- Non-Unix Docker endpoints (`tcp://`, `ssh://`).

## Decisions

### Crate layout
```
cyamus-core      sync   lifecycle, env, paths (+ state home), hooks …
   │ depends on
   ▼
cyamus-registry  sync   record type, label derivation, lock + atomic write, GC, read-all
   ▲
   │ depends on
cyamus-daemon    async  listener, router, proxy, docker source, route table, status API
   ▲
cyamus-cli              clap; `daemon run` enters the tokio runtime; spawn/stop/status client
```
Core calls the registry directly, since it's sync and has no heavy dependencies. Starting the daemon involves spawning `current_exe()`, which is a binary concern, so core takes a small `DaemonControl` trait object from the CLI and calls `ensure_running()` at step 6. Core tests pass a no-op implementation. The tokio runtime is created only inside `cyamus daemon run`, so every other command stays synchronous and starts fast.

### Registry: one file per workspace, read by the daemon on mtime change
The record is TOML, written to a temporary file and then renamed, under an exclusive lock on `registry.lock`. The lock uses `std::fs::File::lock`, stable since Rust 1.89, so no lock crate is needed. The daemon never writes to the registry. On each request, the daemon lists the `workspaces/` tree and keys a cached snapshot on every record file's **name and inode**. It re-reads the records only when that key changes. Directory mtimes were the first idea, but they can miss two changes within one timestamp tick. Every write renames a fresh file into place, so the inode changes on every replacement. With tens of files, a re-read takes microseconds. Garbage collection happens in the CLI (any registry write, plus `daemon status`), never in the daemon, so the daemon stays read-only.

*Alternative:* `notify`-based watching. It's rejected because of per-platform event quirks (FSEvents coalescing, inotify limits) and because an mtime check is cheap at this scale. The user chose this approach during exploration.

### Hostname label is computed once and stored
`branch-label` (the slug truncated to 63 characters) is computed at registration and stored in the record. The daemon never re-derives it, so changing the slug rules later can't silently remap hostnames for workspaces that are already registered.

### Docker: list on events, keep a snapshot
bollard connects to the endpoint resolved from `DOCKER_HOST`, then the current context, then `/var/run/docker.sock`. The current context is read from `~/.docker/config.json` (`currentContext`) and `~/.docker/contexts/meta/<sha256(name)>/meta.json`, without shelling out. The source subscribes to `/events` filtered to container `start`, `die`, `destroy` and `rename`. Each event (debounced by 200 ms) triggers a full `containers list` (running only, with labels and ports), which is turned into an immutable snapshot. Listing everything on each event is simpler and self-healing compared to applying deltas, and it's cheap for local container counts. If the event stream drops, the source reconnects with backoff (max 5 s) and re-lists.

### Route table: pure function, swapped atomically
`routes = resolve(registry_snapshot, docker_snapshot)` is a pure function that returns both the routes and the list of unrouted containers with their reasons. This makes all of the docker-discovery spec testable with in-memory fixtures, with no Docker needed. The result is published through an `ArcSwap`-style handle (or an `RwLock<Arc<_>>`), and the request path only performs an atomic load. Workspace membership uses canonicalized paths on both sides. Paths are canonicalized when the record is written and when the container is seen; on OrbStack the `working_dir` is a host path, so this is a direct comparison.

### Proxy: hyper (spike 1.1: kept)
The listener is a `hyper` HTTP/1.1 server on two `TcpListener`s (`127.0.0.1` and `::1`). Forwarding opens a **fresh loopback connection per request** (`hyper::client::conn::http1`) instead of a pooled client. Containers restart onto the same host ports all the time, so pooled connections would go stale, and loopback connects are effectively free. Upgrades use `hyper::upgrade::on` on both sides, followed by `tokio::io::copy_bidirectional`. Hop-by-hop headers are stripped per RFC 9110, and `Host` is kept, because dev servers such as Vite validate `Host`, and `*.localhost` is accepted by their defaults. Spike 1.1 ran a Vite 6 dev server in a compose stack and confirmed the result. The page loads over IPv4 and IPv6 through `http://web.feat-hmr.vitedemo.localhost:<port>/`. Vite's default `allowedHosts` accepts `*.localhost`. The `vite-hmr` WebSocket connects through the proxy and delivers updates after a file edit. Plain hyper is therefore kept, and `axum-reverse-proxy` isn't needed.

### Status endpoint doubles as the control channel
`cyamus.localhost` is served by the daemon itself. `/api/status` returns JSON with the PID, version, port, Docker state, routes and unrouted containers. The CLI's `daemon status` and setup's version check call it with a tiny blocking HTTP/1.1 client written over `std::net::TcpStream`, which is enough for one GET with `Connection: close`. That avoids pulling an HTTP client crate into the synchronous path. `daemon stop` uses SIGTERM to the PID in `daemon.pid`, then polls until the PID exits (timeout 5 s).

*Alternative:* a Unix control socket. It's rejected because it would be a second server, and the HTTP endpoint is useful in the browser anyway.

### Crate dependencies, as built
`cyamus-core` depends on `cyamus-registry`, not the other way round, so the registry takes an already-computed slug (`label_from_slug`) instead of calling core's `slugify`. `cyamus-daemon` depends on both, because it needs `Dirs` and the registry.

### Starting the daemon
`ensure_running()`:
1. Probe `/api/status` on the configured port (500 ms read timeout). If the probe succeeds and the version matches, return.
2. Take the blocking **`daemon.spawn.lock`**, then probe again, because a racing setup may have just started the daemon. Spawners serialize on this separate lock because the daemon itself holds `daemon.lock` for its whole life, and a spawner holding that lock would block its own child.
3. If the probe finds a different version, SIGTERM the PID it reports and wait for it to exit.
4. If nothing answers but `daemon.lock` is held, a daemon is running on another port. Fail with a message naming the port mismatch.
5. Otherwise spawn `current_exe() daemon run --background`, with stdin from `/dev/null` and stdout/stderr appended to `daemon.log` (truncated first if over 10 MiB). The hidden `--background` flag makes the child call `setsid()` itself. The child isn't a process-group leader, so this succeeds without `pre_exec`, which would need `unsafe`, and `unsafe` is forbidden in this workspace.
6. Poll the probe for up to 3 s, failing early if the child exits, and report the last log line.

Any failure produces a warning that points to `daemon.log`. Signals and liveness checks use `rustix` (pure Rust).

### Hook environment additions
- `COMPOSE_PROJECT_NAME = <project>-<branch-label>`. Both parts are DNS-safe, so the result satisfies compose's `^[a-z0-9][a-z0-9_-]*$`. It's set only if absent from the parent environment, so a user override in Orca's script wins.
- `CYAMUS_DOMAIN` and `CYAMUS_PROXY_PORT` let a compose file interpolate public URLs, for example `APP_URL: http://web.${CYAMUS_DOMAIN}:${CYAMUS_PROXY_PORT}` for CORS or OAuth callbacks.
- These variables only exist for hook-run compose. A manual `docker compose up` in a terminal won't have them, so it gets the default project name and a second stack. The README documents this. The templating change fixes it properly with `name:` in the asset.

### Port 1355, loopback only, env-configurable
`1355` sits below every OS's ephemeral range, so Docker never hands it out to a published container. It isn't a common dev-server default either. `CYAMUS_DAEMON_PORT` is the only setting, and one setting doesn't justify a global config file yet. Setup and the daemon must agree on the port: setup reads the same variable, and `daemon status` prints the running daemon's port. If they disagree, setup reports that a daemon holds `daemon.lock` but isn't answering on its port.

### Port 80: a loopback-only relay, installed explicitly
The user wants URLs without a port. Exploration on the target Mac found these constraints:
- An unprivileged process can bind `:80` only on wildcard addresses (`0.0.0.0` or `::`), not on `127.0.0.1` or `::1`.
- The macOS Application Firewall is on. A wildcard listener in an ad-hoc-signed binary makes macOS ask whether to accept incoming connections, and probably again after every upgrade.
- A wildcard listener is reachable from the LAN.

So the daemon never listens on 80.

**First attempt, abandoned: a kernel redirect.** A `pf` rdr rule loaded into the `com.apple/cyamus` sub-anchor on macOS, and an `nftables` NAT `output` rule on Linux. The nft version worked in a privileged Linux container. On the user's Mac, pf had already been enabled for 9 days by other software, which had set **`set skip on lo0`**. pf then ignores all loopback traffic, so no rdr rule can apply (`pfctl -sI -v` shows `lo0 (skip)`). Overriding a global pf option owned by another tool would be fragile and invasive, and Linux firewall managers that flush rulesets pose the same kind of risk to nft. Kernel redirection is rejected.

**Chosen: `cyamus daemon relay`, a byte-level TCP forwarder run as a system service.** `cyamus daemon install` sets it up:

- **What it does.** Listens on `127.0.0.1:80` and `[::1]:80` only, which avoids firewall prompts and LAN exposure. Forwards each connection to the same loopback address on the daemon port, with `tokio::io::copy_bidirectional`. It never parses HTTP. The daemon sees the original `Host` header, but the peer address becomes the relay's (loopback anyway).
- **No root code in a user-writable path.** Install copies the running binary to a root-owned **`/usr/local/libexec/cyamus-relay`** (`install -m 755`), and the service runs that copy. Pointing a root LaunchDaemon at `~/.local/bin/cyamus` would let anything running as the user get root at the next boot. The relay is protocol-agnostic, so a stale copy after a cyamus upgrade is harmless; reinstalling refreshes it.
- **macOS.** A LaunchDaemon, `/Library/LaunchDaemons/dev.cyamus.relay.plist`, with `RunAtLoad` and `KeepAlive`, logging to `/var/log/cyamus-relay.log`, runs `cyamus-relay daemon relay --to <port>` as root. Binding loopback port 80 needs root, and macOS has no capabilities. Dropping root without `unsafe` (forbidden here) and without `nix` works like this:
  - `nix`'s `setgroups` and `initgroups` aren't available on Apple targets, so an in-process `setuid` would keep root's supplementary groups.
  - The first build spawned the child with std's `CommandExt::{uid, gid}`. The macOS verification (8.5) showed the child with uid `nobody` but **effective and saved gid 0**. std calls `setgid(gid)` and then `setgroups(0, NULL)`, and on Darwin the first group-list entry *is* the effective gid, so an empty `setgroups` resets it to 0. With no safe `setregid` on Apple (in `rustix` or `nix`), the child can't repair this itself.
  - So the root process binds both sockets and then **`exec`s `sudo -n -u '#<uid>'`**, which runs `cyamus-relay daemon relay --to <port> --inherited`. As root, sudo never prompts, and it sets the real, effective and saved ids and the group list through the platform's user context. Only `-u` is passed: macOS's default `root ALL = (ALL) ALL` rule allows choosing the user, while `-g` would need `(ALL:ALL)`, and `nobody`'s primary group comes from its user record anyway. Because of `exec`, no Rust process stays root; the remaining root process is sudo's own monitor, which relays signals to the child.
  - The listening sockets are passed as the child's **stdin and stdout**, inetd-style; sudo keeps fds 0–2. The child turns them back into `TcpListener`s with `std::io::stdin().as_fd().try_clone_to_owned()`, which is safe.
  - **Fail closed:** before serving, the child checks `getuid`/`geteuid`/`getgid`/`getegid` and `getgroups` (via rustix) and exits if any of them is 0.
  - `nobody`'s uid is looked up with `id -u nobody` at install time and passed as `--user <uid>`.
- **Linux.** A systemd unit, `/etc/systemd/system/cyamus-relay.service`, with `DynamicUser=yes` (an ephemeral user; `systemd-analyze verify` flags `User=nobody` as unsafe, because `nobody` is shared by unrelated services), `AmbientCapabilities=CAP_NET_BIND_SERVICE`, `CapabilityBoundingSet=CAP_NET_BIND_SERVICE`, `NoNewPrivileges=yes` and `Restart=always`. The relay never holds root, and it binds and serves directly.
- **Same code on both.** `daemon relay` binds, then either serves (when not root, or when given inherited sockets) or spawns the unprivileged child (when root and given `--user`). The serving path is plain user-level code, tested end to end on high ports.
- **Running privileged steps.** The command runs as the user and invokes `sudo` for each privileged step, printing the command first. Files are staged in a temp directory and placed with `sudo install`. `--dry-run` prints the same plan. Under `sudo` (euid 0), install sets up the relay but does **not** start a daemon, because that daemon would run as root with root's state directory. It verifies only if the user's daemon is already up, and otherwise explains what to do.
- **Uninstall.** Stops and removes the service and the binary copy. On macOS, install and uninstall also remove the abandoned pf attempt's `dev.cyamus.redirect` job, `com.apple/cyamus` anchor and files, if present.
- **Detection without root.** `cyamus daemon status` and setup's `CYAMUS_URL_SUFFIX` probe `127.0.0.1:80` for `cyamus.localhost/api/status` and compare the PID. This works the same through the relay, or through anything else that forwards port 80.
- **Generated URLs.** The route page and the 404 page build their links from the `Host` header of the request they're answering.

*Alternatives:*
- Kernel redirect (pf/nft): rejected, see above.
- Wildcard bind with a peer filter: rejected for the firewall prompts and the LAN exposure.
- launchd socket activation: would need `launch_activate_socket` through FFI (`unsafe`).
- Lowering `net.inet.ip.portrange.reservedhigh` (macOS) or `ip_unprivileged_port_start` (Linux): a system-wide relaxation that lets any process bind low ports.

## Risks / Trade-offs

- [The compose `working_dir` is the directory of the compose file as invoked, not resolved through symlinks.] → Spike 1.2 (compose 5.1.2, OrbStack) confirmed that a `[[link]]`ed `compose.yaml` records the *worktree* as `working_dir`, and that `-f infra/compose.yaml` records `<worktree>/infra`. Both copy and link work, and longest-prefix matching covers subdirectories. Only `-f` pointing outside the worktree escapes, and explicit labels are the escape hatch. Both sides are canonicalized, because compose may record a non-canonical `$PWD` path.
- [A manual `docker compose up` creates a second stack with the default project name.] → Documented. Both stacks' containers map to the same workspace, so their hostnames collide, and "duplicate" reporting makes it visible. Fixed properly by templating.
- [A stale daemon after an upgrade, if the user never runs setup.] → `daemon status` shows the version. The next setup hands over.
- [Docker events missed while disconnected.] → A full re-list on every reconnect.
- [WebSocket or streaming edge cases in a hand-written hyper proxy.] → Integration tests against real local backends (SSE, WebSocket echo, large bodies). The `axum-reverse-proxy` fallback has already been agreed.
- [The daemon log grows forever.] → On start, the log is truncated if it is over 10 MiB. No rotation.
- [The macOS relay keeps a root process for its whole life: sudo's monitor.] → It never reads network data; it relays signals to the child. The child that handles connections is `nobody`, and refuses to serve if any root identity remains.
- [The relay forwards to the port in use at install time.] → Changing `CYAMUS_DAEMON_PORT` later means running install again. Status shows that port 80 doesn't reach the running daemon, because the PID check fails.
- [The background daemon inherits any file descriptor its caller left without close-on-exec, and keeps it open for its whole life. If that is a pipe the caller reads to EOF, the caller hangs.] → Orca spawns through libuv, which uses close-on-exec as the default on macOS, so setup only gets 0–2. The test suite hit this by spawning setups from several threads at once: std on macOS sets `FD_CLOEXEC` after `pipe()`, not atomically. The test now spawns from one thread. Closing arbitrary inherited descriptors in the daemon needs `unsafe` (`close_range`/`BorrowedFd::borrow_raw`), which the workspace forbids; revisit with an audited exception if a real caller hits it.
- [Hostname collisions inside a project when two branches slug the same.] → The first one registered keeps the label, and the second gets a warning (see the workspace-registry spec).

## Migration Plan

This is purely additive. Existing projects keep working, and the only visible change is a background daemon started by setup, which can be disabled with `CYAMUS_DAEMON=off`. The README gains "Daemon and routing" and "Docker" sections.

## Future work (not in this change)

### HTTPS
What would be needed:
- **Local CA.** On first use, `cyamus daemon install` generates a CA key and certificate (`rcgen`) under `$XDG_STATE_HOME/cyamus/ca/` (key mode 0600).
- **Trust.** Trusting the CA needs privileges and an interactive step, so it lives in that explicit install command and never in setup:
  - macOS: `security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db`.
  - Linux: copy to `/usr/local/share/ca-certificates` and run `update-ca-certificates`, plus a per-browser NSS database (`certutil`) for Firefox and Chromium on Linux.
  - Node does not use the system store, so the install command should print `NODE_EXTRA_CA_CERTS=<ca.pem>`.
- **Per-host certificates.** Issue certificates on demand per SNI in a `rustls` `ResolvesServerCert`, caching them in memory. Use `*.<branch>.<project>.localhost` wildcards to bound certificate count. Wildcards only cover one label, so a certificate per workspace is needed.
- **TLS stack.** Use rustls with the **`ring`** provider, not `aws-lc-rs`, to keep the musl release builds free of cmake and C. Use `tokio-rustls` for the acceptor. ALPN gives h2 for free with hyper-util's auto builder.
- **Listener and headers.** A second listener (`CYAMUS_DAEMON_TLS_PORT`, default 1356), and `X-Forwarded-Proto: https` on that listener.

### Port 443
Port 443 works like port 80: the relay gains a second listen/target pair (443 → `CYAMUS_DAEMON_TLS_PORT`). It stays a byte-level forwarder, so TLS terminates in the daemon.

### `[[service]]` port allocation
- For dev servers run on the host (`npm run dev`). The manifest declares service names, setup allocates a stable machine-wide port per (workspace, service) from a range, records it in the same registry record (`[ports]` table), and exposes `CYAMUS_PORT_<SVC>`.
- The daemon routes these exactly like containers, with the source being the registry instead of Docker.
- The registry layout in this change leaves room for this: one record per workspace, with the CLI as the only writer.

### `cyamus compose`
- Project-shared services (db, redis) from `<config>/compose.yaml`, run with `docker compose -p <project> -f <config>/compose.yaml`, as in git-workspace.
- Its containers have `working_dir` set to the config directory, so this change correctly leaves them unrouted.
- A later change can route them as `<svc>.<project>.localhost`, a hostname shape reserved by the proxy-routing spec.

### Templated compose project name
With the `templating` change, a compose asset can declare `name: {{ project }}-{{ branch_slug }}`. That makes manual and hook-run compose agree, and lets the `COMPOSE_PROJECT_NAME` export become redundant.

## Open Questions

- Should the route page (`cyamus.localhost`) also offer actions (restart a container, open logs)? This can be decided later; it doesn't change the specs or the structure here.
