# Tasks

## 1. Spikes

- [x] 1.1 Proxy spike: a throwaway hyper + hyper-util program that forwards HTTP/1.1, streams SSE, and passes a WebSocket upgrade through to a Vite dev server with working HMR. Verify that hot reload works through the proxy in a browser. Record in design.md whether hyper is kept or `axum-reverse-proxy` is adopted.
- [x] 1.2 Compose `working_dir` spike: run `docker compose up` (a) with a copied `compose.yaml`, (b) with a symlinked one, (c) with `-f` from a subdirectory, and record `com.docker.compose.project.working_dir` for each. Verify that the findings match the docker-discovery spec, and update the README guidance and design risks if (b) or (c) differs.
- [x] 1.3 Docker endpoint spike: resolve the current context endpoint from `~/.docker` metadata and connect bollard without default TLS features. Verify on OrbStack that containers are listed and that `cargo tree` shows no openssl or aws-lc.

## 2. State paths and registry crate

- [x] 2.1 Add `XDG_STATE_HOME` (default `~/.local/state`) to `Dirs` in core, with helpers for `workspaces/`, `registry.lock`, `daemon.{pid,lock,log}`. Verify with unit tests for the env and default resolution.
- [x] 2.2 Create the `cyamus-registry` crate: record type (project, branch, label, canonical path), label derivation (slug truncated to 63 characters, trailing `-` trimmed, empty means none), and TOML (de)serialization. Verify with unit tests for long, empty and normal branches.
- [x] 2.3 Implement `register`, `unregister` and `gc`: lock on `registry.lock`, temp-file-and-rename writes, idempotent re-registration, conflict detection (existing live path vs stale), removal only when the record's path matches, and cleanup of empty project directories. Verify with unit tests for every workspace-registry scenario, including a multi-threaded concurrent-register test.
- [x] 2.4 Implement `read_all` and a cheap change key (max mtime over the tree). Verify with a unit test that the key changes after register and after unregister.

## 3. Lifecycle integration

- [x] 3.1 Add the `DaemonControl` trait to core and wire steps 5–6 into `lifecycle::setup`: register, warn on conflict or empty label, then call `ensure_running()` and warn on failure. Verify with integration tests in `crates/cyamus-cli/tests/lifecycle.rs` that the record exists before hooks run (a hook reads the registry file), and that a failing `DaemonControl` still lets hooks run with exit 0. (The failing-daemon case is covered end to end by 6.4.)
- [x] 3.2 Wire unregistration into `lifecycle::teardown` after hooks, including when hooks fail. Verify with integration tests for both a successful teardown and a failing hook (record removed, non-zero exit).
- [x] 3.3 Add `CYAMUS_DOMAIN`, `CYAMUS_PROXY_PORT` and `COMPOSE_PROJECT_NAME` (unless already set) to the hook environment. Verify with the env scenarios from the modified workspace-lifecycle spec in `tests/hooks.rs`, including the caller-override case.
- [x] 3.4 Update the README "Environment" table with the three new variables. Verify that `tests/readme.rs` still passes.

## 4. Daemon crate: routing core

- [x] 4.1 Create the `cyamus-daemon` crate with the tokio, hyper and bollard dependencies (no TLS features). Verify that `cargo tree -p cyamus-daemon -i openssl-sys` and `-i aws-lc-sys` find nothing, and that `cyamus-core` still has no tokio dependency (`cargo tree -p cyamus-core -i tokio` is empty).
- [x] 4.2 Implement a pure `resolve(registry, containers) -> (routes, unrouted)`:
  - membership (explicit labels, then longest-prefix `working_dir`);
  - opt-out;
  - service naming and sanitizing;
  - port selection;
  - published IP mapping;
  - duplicates (earliest start wins);
  - invalid-label reasons.

  Verify with table-driven unit tests covering every docker-discovery scenario, using in-memory fixtures.
- [x] 4.3 Implement Host matching (case-insensitive, port and trailing dot ignored, `cyamus.localhost` reserved, two-label project hosts never produced). Verify with unit tests for the proxy-routing hostname scenarios.

## 5. Daemon crate: sources and server

- [x] 5.1 Registry source: a snapshot re-read on change-key change, checked per request. Verify with a unit test that a record written after startup is routed on the next resolve, without a restart.
- [x] 5.2 Docker source:
  - endpoint resolution (`DOCKER_HOST`, then current context, then `/var/run/docker.sock`);
  - running-container listing mapped to fixtures;
  - event subscription with 200 ms debounce;
  - reconnect with backoff of at most 5 s, then re-list.

  Verify with unit tests for endpoint resolution, and with an ignored-by-default integration test (`CYAMUS_DOCKER_TESTS=1`) that starts a labeled container and sees a route within 2 s.
- [x] 5.3 Server:
  - dual loopback listeners on `CYAMUS_DAEMON_PORT` (default 1355), tolerating one bind failure;
  - forwarding with hop-by-hop stripping, Host preserved and `X-Forwarded-*` added;
  - streaming bodies and WebSocket upgrade relay;
  - 404 (HTML or text) listing routes, and 502 naming the target.

  Verify with integration tests against local tokio backends: plain request with headers asserted, SSE incremental delivery, WebSocket echo, unknown host, and refused backend.
- [x] 5.4 `cyamus.localhost` route page (HTML) and `/api/status` JSON (PID, version, port, Docker state, routes, unrouted). Verify with integration tests asserting the JSON shape and that the HTML lists routes as links.
- [x] 5.5 Lifecycle:
  - single-instance `daemon.lock`;
  - `daemon.pid` written and removed;
  - SIGINT/SIGTERM graceful shutdown;
  - log truncation over 10 MiB.

  Verify with an integration test: run, then run again (second exits non-zero with the PID), then SIGTERM (exit 0, pid file gone).

## 6. CLI

- [x] 6.1 Add `cyamus daemon run | status | stop` to clap. `run` builds the tokio runtime and runs the daemon in the foreground. Verify with `cyamus daemon --help` and an e2e test that `run` serves `/api/status`.
- [x] 6.2 Blocking status client over `std::net::TcpStream`, and `daemon status` output (running/not running, routes, unrouted reasons, Docker reachability), running registry GC first. Verify with e2e tests for exit 0 with a route listed and for exit 1 when not running.
- [x] 6.3 `daemon stop`: SIGTERM to the PID from `daemon.pid`, then wait up to 5 s. With no daemon running, print a notice and exit 0. Verify with an e2e test that the port is free after stop.
- [x] 6.4 `DaemonControl` implementation: probe, version handover, lock-guarded spawn of `current_exe() daemon run` with `setsid`, `/dev/null` stdin and appended `daemon.log`, a 3 s readiness poll, and `CYAMUS_DAEMON=off`. Verify with e2e tests:
  - setup starts a daemon that survives setup's exit;
  - three parallel setups leave one daemon;
  - `CYAMUS_DAEMON=off` starts none;
  - a held port gives a warning and exit 0.
- [x] 6.5 Isolate daemon e2e tests: each test uses its own `XDG_STATE_HOME` and a free `CYAMUS_DAEMON_PORT`, and stops its daemon on drop. Verify that `devbox run test` passes twice in a row with no leftover `cyamus daemon` processes (`pgrep -f "cyamus daemon"` is empty).

## 7. Documentation

- [x] 7.1 Add a README "Daemon and routing" section:
  - hostname scheme;
  - port 1355 and `CYAMUS_DAEMON_PORT`;
  - `daemon status/stop/run`;
  - `http://cyamus.localhost:1355`;
  - `CYAMUS_DAEMON=off`;
  - loopback only and no HTTPS yet.

  Verify that the documented commands and URLs match the implementation and that `tests/readme.rs` passes.
- [x] 7.2 Add a README "Docker" section:
  - the compose-as-copied-asset example (manifest + asset + hooks);
  - `compose.override.yaml` for repos that already track a compose file;
  - ephemeral port publishing;
  - a `cyamus.*` label reference table;
  - the caveat about running compose by hand;
  - container-to-container calls via service names.

  Verify the example end to end in 9.1.

## 8. Port-80 relay

- [x] 8.1 Relay plan module (replaces the pf/nft plan): for a port and a source binary, generate the macOS LaunchDaemon plist (root, `--user <nobody uid:gid>`, KeepAlive) or the Linux systemd unit (`DynamicUser=yes`, `CAP_NET_BIND_SERVICE` only, `NoNewPrivileges`), the binary copy to `/usr/local/libexec/cyamus-relay`, and the ordered install and uninstall commands, including cleanup of the abandoned pf attempt on macOS. Verify with unit tests asserting the plist/unit content and that no step disables pf or nftables.
- [x] 8.2 `cyamus daemon relay --to <port> [--listen <port>] [--user <uid:gid>]`: bind loopback v4/v6. As root with `--user`, spawn the serving child as that user (std `uid`/`gid`, groups cleared) with the sockets passed as stdin/stdout, and wait on it. Otherwise serve: `copy_bidirectional` to the same loopback family on `--to`, closing clients when the target refuses. Verify with e2e tests on high ports: an HTTP request through the relay reaches a running test daemon with its `Host` intact; the inherited-socket child path (without a uid change) serves too; a refused target closes the client and the relay survives.
- [x] 8.3 `cyamus daemon install/uninstall [--dry-run]` on top of the plan: tool checks (missing tools only noted in a dry run), `sudo` per step with the command echoed, a root-owned binary copy, no daemon start when run under `sudo`, and verification by PID. Detection, `CYAMUS_URL_SUFFIX`, status output, and links from the `Host` port were done in the previous iteration (kept). Verify with dry-run e2e tests and the existing detection/status tests.
- [x] 8.4 Linux verification: in a container, run the musl `cyamus daemon relay --listen 80 --to 1355` as `nobody` with only `CAP_NET_BIND_SERVICE` (`setpriv` or `capsh`), next to `cyamus daemon run`. Confirm `curl http://cyamus.localhost/` over v4 and v6 reaches the daemon, that the relay process is `nobody`, and that `daemon status` reports port 80. Also confirm the generated systemd unit passes `systemd-analyze verify`. Record the output here.
  - Result (Debian stable-slim, aarch64 musl build):
    - `setpriv` ran `/usr/local/libexec/cyamus-relay daemon relay --to 1355` as `nobody` with only `CAP_NET_BIND_SERVICE` (inheritable, ambient and bounding sets). `ps` showed user `nobody`.
    - `curl -4` and `curl -6` to `http://cyamus.localhost/api/status` returned 200 via `127.0.0.1:80` and `::1:80`, and `daemon status` reported "port 80: redirected to the daemon".
    - The container's own network address on port 80 was refused.
    - `systemd-analyze verify` flagged `User=nobody` as unsafe, so the unit now uses `DynamicUser=yes` and verifies without warnings.
    - The unit was not run under a real systemd PID 1 (containers).
- [x] 8.5 macOS verification: the user runs `cyamus daemon install` (without `sudo`; it prompts). Confirm `http://cyamus.localhost/` and the fixture routes answer without a port, the serving relay process runs as `nobody`, `daemon status` reports port 80, the pf leftovers are gone, and `uninstall` then `install` round-trips. Record the output here.
  - Result (macOS 26.6.2, OrbStack, pf enabled by other software with `set skip on lo0`):
    - The first relay build dropped privileges with std `CommandExt::uid/gid`. The serving child then had uid `nobody`, but effective and saved gid 0, because Darwin's `setgroups(0, NULL)` resets the effective gid. It was fixed by exec'ing `sudo -n -u '#<uid>'` and adding a fail-closed check of the ids before serving.
    - After `uninstall` and `install`, the processes are root `sudo -n -u #4294967294 -- /usr/local/libexec/cyamus-relay daemon relay --to 1355 --inherited` (sudo's monitor), with child pid 25045 serving. The child's uid/ruid/svuid and gid/rgid/svgid are all -2 (`nobody`), and it passed its fail-closed group check.
    - `/usr/local/libexec/cyamus-relay` is root:wheel 755, and its md5 matches the installed build.
    - The relay log shows SIGTERM on uninstall and a fresh start on install.
    - `http://cyamus.localhost/`, `http://web.feat-hmr.vitedemo.localhost/` and `http://web.feat-two.vitedemo.localhost/` answered 200 via `::1:80`; `/api/status` answered via `127.0.0.1:80`.
    - `daemon status` reported "port 80: redirected to the daemon", and the LAN address `10.0.0.7:80` was refused.
    - The leftovers from the pf attempt (`dev.cyamus.redirect`, `/etc/pf.anchors/cyamus`) were removed by install.
- [x] 8.6 README: describe the relay instead of pf/nft (what install changes on each OS, the root-owned copy, `nobody`), and keep `tests/readme.rs` in step. Verify that the README test passes.

## 9. End-to-end validation

- [x] 9.1 On macOS with OrbStack, configure a real project with the README's compose example. Run setup on two worktrees and confirm both stacks are routed at their own port-less `*.localhost` hostnames in a browser, including HMR through the proxy. Confirm `daemon status` lists both, teardown of one removes its routes within 2 s, and `cyamus.enable=false` hides the db.
  - Result: the test project `vitedemo` used a compose asset (Vite in `node:22-alpine`, plus `db` with `cyamus.enable: "false"`) on worktrees `feat/hmr` and `feat/two`.
    - Both were routed at `http://web.<branch>.vitedemo.localhost/` (200 through the port-80 relay), and `daemon status` listed both, with both `db`s reported as opted out.
    - The Vite HMR WebSocket through `ws://web.feat-hmr.vitedemo.localhost/` received `connected` and then `full-reload` after `main.js` was edited.
    - Tearing down `feat/two` removed its route 1.45 s after the teardown command started (compose down included).
    - Hooks saw `CYAMUS_URL_SUFFIX=` (empty), `COMPOSE_PROJECT_NAME=vitedemo-feat-two`, and `:1355` with `CYAMUS_DAEMON=off`.
    - A 502 right after `compose up`, while Vite installed, became 200 within about 1 s.
    - Not done: the visual browser check, because the Chrome extension was not connected. HTTP and WebSocket were verified with curl and Node's WebSocket client instead.
- [x] 9.2 Run `devbox run check` and confirm the release workflow's musl targets still build (`cargo build --release --target x86_64-unknown-linux-musl` in the Docker container used before). Verify that both succeed.
  - Result: `devbox run check` passed (19 test binaries) on repeated runs. `x86_64-unknown-linux-musl` (amd64 container) and `aarch64-unknown-linux-musl` both built as static binaries, and `cyamus --version` and `daemon --help` ran.
