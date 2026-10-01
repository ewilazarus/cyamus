# Tasks

## 1. Spikes

- [x] 1.1 Compose file discovery: on Compose 5.1.2, record which files `docker compose config` picks for each case: each default name alone; two names together (preference order); an `.override` sibling; a file only in a parent directory (upward search, and where it stops); `COMPOSE_FILE` with several entries. Verify by writing the observed table into design.md and adjusting the discovery rules if they differ.
- [x] 1.2 Resolved model: from `docker compose config --format json`, record how implicit `default` networks, explicit networks with aliases, `network_mode`, `depends_on` entries with `required`/`restart`, and services behind an inactive profile appear, and which flags include profiled services. Verify by writing the findings into design.md.
- [x] 1.3 End-to-end override by hand: write the override design.md describes for a two-stack example (shared `db` plus `mailpit`; worktree `web`→`db`+`cache`, a `backend` custom network, and a `network_mode: host` service). Confirm with plain `docker compose` that `web` reaches the shared `db` by name, `cache` still gates `web`, the custom network is kept, and both stacks' `down` succeed in either order. Verify by recording the commands and results here.
  - Result: the override mechanics work (see design.md). The project-wide network caused cross-worktree DNS answers, so the design moved to per-worktree networks.
- [x] 1.4 Re-run 1.3 in the per-worktree-network shape with **two** worktrees:
  - shared containers connected with `--alias` before each worktree's `up`;
  - from each worktree, 20 lookups of a name both define always return its own service, and a name only the other defines doesn't resolve;
  - `web` reaches the shared `db` at first start;
  - after `--force-recreate` of the shared `db` plus a reconcile, it's reachable again;
  - `down` and network removal work in either order.

  Verify by recording the results here.
  - Result (OrbStack, Compose 5.1.2, worktrees `a` and `b`, shared `db`):
    - Both `web`s got `shared-db` on their **first** connection at start-up, and neither worktree started its own `db`.
    - 20 out of 20 `api` lookups returned the worktree's own `api` in both `a` and `b`, and `a` could not resolve `only-b`.
    - After `--force-recreate` of the shared stack, `db` was unresolvable (`bad address 'db'`) until the reconcile, and reachable right after it.
    - Shared `down` with worktrees up: ok. Worktree `a` `down` with shared containers attached: ok. Then disconnect and `network rm`: ok, and `b` still reached `db`.
    - Everything was cleaned up afterwards.

## 2. Core: compose invocation

- [x] 2.1 Add `cyamus_core::compose` with argument splitting (global options vs. subcommand, every `-f`/`--file` form) and file discovery (defaults with upward search to `/`, compose's base and override preference orders, `COMPOSE_FILE`). Verify with unit tests for each rule from spike 1.1 and each `-f` form.
- [x] 2.2 Model reading: run `docker compose … config --format json` through a small `Docker` runner trait (fakeable in tests) and parse services, `depends_on`, networks and `network_mode`. Verify with unit tests on JSON fixtures captured in spike 1.2.
- [x] 2.3 Override generation for the worktree stack: shadowed profile, `depends_on: !override` minus shadowed entries, and `networks: !override` (own networks plus `cyamus-net`, with `network_mode` skipped), all as YAML with JSON-flow values. Also the external `cyamus-net` declaration. Verify with golden-text unit tests, and by loading each golden override with real `docker compose config` in an ignored-by-default test (`CYAMUS_DOCKER_TESTS=1`).
- [x] 2.4 Worktree network operations: ensure (inspect, create with both labels, tolerate a creation race); attach the running shared containers with `--alias <service>`, skipping existing attachments; reconcile across every network labelled with the project; detach and remove on `down`. Plus the Compose version check (≥ 2.24.4, clear error). Verify with unit tests through the fake runner, plus the Docker-gated test.
- [x] 2.5 Environments for both stacks: worktree (full hook variables minus `CYAMUS_EVENT`, from the manifest and fingerprints, added only where missing) and shared (project-level values set explicitly, `CYAMUS_DOMAIN=<project>.localhost`, worktree variables, `CYAMUS_EVENT`, fingerprints, inherited `CYAMUS_VAR_*` and `COMPOSE_PROJECT_NAME` removed). Verify with unit tests: an inherited runtime `CYAMUS_VAR_*` wins for the worktree stack and is replaced for the shared stack; branch variables are absent from the shared environment.

## 3. CLI commands

- [x] 3.1 `cyamus compose-shared [args…]`: resolve the project from the current directory, require `<config>/compose.yaml`, run `docker compose -p <project> -f <config>/compose.yaml <args>` in the config directory with the shared environment, then reconcile the shared containers onto the project's worktree networks. Pass the exit code through. Verify with e2e tests using a fake `docker` on `PATH` that records its argv, environment and working directory.
- [x] 3.2 `cyamus compose [args…]`: worktree resolution, the label requirement, project name (`COMPOSE_PROJECT_NAME` or `<project>-<label>`), file selection, shadowing against the shared stack (when the file exists), the override, the worktree environment (including `CYAMUS_URL_SUFFIX` from port-80 detection), and a stderr notice listing shadowed services. Verify with fake-`docker` e2e tests for each compose-stacks scenario that doesn't need real containers.
- [x] 3.3 Shared stack on `up`/`run`: running-services check, `up -d` with a notice, stop on failure, then attach to this worktree's network before the worktree's compose runs. No effect for other subcommands. After a successful `down`, detach and remove the worktree network. Verify with fake-`docker` e2e tests (not running, running, failing, down cleanup) and that `down` never calls the shared stack.

## 4. Daemon: shared-stack routes

- [x] 4.1 Make `Route.workspace` optional (status JSON, page, CLI rendering), add project config directories to `resolve`, and route matching containers as `<svc>.<project>.localhost` with every existing rule applied. Verify with table-driven unit tests (match, opt-out, a non-matching directory, explicit labels win) and the proxy test for a shared hostname.
- [x] 4.2 Feed the config directories into the daemon state (listed from `$XDG_CONFIG_HOME/cyamus/projects`, refreshed with the registry key). Verify with a state unit test that a newly created project directory is picked up without a restart.

## 5. Documentation

- [x] 5.1 README "Docker" section:
  - `cyamus compose` and `cyamus compose-shared`, with shadowing explained by example;
  - hooks switched to `cyamus compose up -d` / `cyamus compose down`;
  - "shared services keep running; stop them with `cyamus compose-shared down`";
  - shared routes (`<svc>.<project>.localhost`);
  - the Compose ≥ 2.24.4 requirement.

  Verify with `tests/readme.rs` (extended to the new commands).
- [x] 5.2 README worked example: one shared Postgres with a database per worktree. Show the shared `compose.yaml` with `postgres`, a setup hook that creates `app_${CYAMUS_BRANCH_SNAKE}` idempotently via `cyamus compose-shared exec`, a teardown hook that drops it, and the app's `DATABASE_URL` pointing at `db` with that name. Verify by running the example end to end in 6.1.

## 6. End-to-end validation

- [x] 6.1 On OrbStack with a real project:
  - The README Postgres example: `compose-shared` stack with `postgres` and `mailpit`; a worktree compose with its own `db` (shadowed) and `web`.
  - Set up two worktrees through hooks and confirm: one Postgres container total; each worktree's `web` connects to its own database by name `db`; `mailpit.<project>.localhost` routes; the first `cyamus compose up` started the shared stack and the second didn't.
  - Tearing down one worktree drops its database and leaves the shared stack running.
  - `cyamus compose-shared down` succeeds while the other worktree runs.

  Verify by recording the observed output here.
  - Result (OrbStack, Compose 5.1.2, project `pgdemo`, the README example with hooks `cyamus compose-shared up -d --wait`, `create_db`, `cyamus compose up -d` / `cyamus compose down`, `drop_db`):
    - Setting up `feat/a` and `feat/b` printed "using shared: db". Exactly one Postgres container ran for the project (`pgdemo-db-1`; the `web`s only use the image as a client).
    - `web` in each worktree connected to `db` by name and landed in `app_feat_a` and `app_feat_b` respectively. Both databases existed in the shared Postgres, with one network per worktree (`cyamus-pgdemo-feat-a`, `-feat-b`).
    - `http://mailpit.pgdemo.localhost/` returned 200 through the port-80 relay, and status showed the shared `db` as "(pgdemo/shared): opted out".
    - Tearing down `feat/a` dropped `app_feat_a`, removed its containers and network, and left the shared stack running.
    - `cyamus compose-shared down` succeeded while `feat/b` ran (its `web` and network were unaffected). A later `cyamus compose up -d` in `feat/b` printed "starting the shared stack for pgdemo", and `web` reached `app_feat_b` again.
    - Everything was cleaned up afterwards (no pgdemo containers or networks).
- [x] 6.2 Run `devbox run check` twice and the musl release build. Verify that both succeed with no leftover processes.
  - Result: `devbox run check` passed twice (21 test binaries). The Docker-gated suites (`compose_docker`, `docker`) passed against OrbStack. The `aarch64-unknown-linux-musl` release build succeeded and lists both commands. No leftover daemons or test networks.
