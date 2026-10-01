# Spec Delta

## Purpose

Defines `cyamus compose` and `cyamus compose-shared`: running a worktree's compose stack and the project's shared stack side by side, with services that exist in the shared stack used from there instead of being duplicated per worktree.

## ADDED Requirements

### Requirement: Shared stack command
`cyamus compose-shared [args…]`, run from anywhere inside a worktree, SHALL run `docker compose` with:
- project name `<project>`;
- `<config>/compose.yaml` as the compose file;
- the project config directory as the working directory;
- the remaining arguments passed through unchanged.

It SHALL exit with `docker compose`'s exit code. If `<config>/compose.yaml` does not exist, it SHALL fail and name that path.

#### Scenario: Start the shared stack
- **WHEN** a user runs `cyamus compose-shared up -d` in a worktree of `myproj`
- **THEN** `docker compose -p myproj -f ~/.config/cyamus/projects/myproj/compose.yaml up -d` runs in the config directory, and the shared containers are then attached to the project's worktree networks

#### Scenario: No shared compose file
- **WHEN** `<config>/compose.yaml` does not exist
- **THEN** the command exits non-zero with an error naming the expected path

### Requirement: Worktree stack command
`cyamus compose [args…]`, run from anywhere inside a worktree, SHALL run `docker compose` with:
- project name `COMPOSE_PROJECT_NAME` if set, otherwise `<project>-<branch-label>`;
- the compose files described in "Compose file selection", followed by cyamus's generated override;
- the remaining arguments passed through unchanged.

It SHALL exit with `docker compose`'s exit code. Outside a worktree, or on a branch without a usable label, it SHALL fail with an error.

#### Scenario: Bring up the worktree stack
- **WHEN** a user runs `cyamus compose up -d` in worktree `feat/x` of `myproj`
- **THEN** compose runs with project name `myproj-feat-x`

#### Scenario: Exit code passes through
- **WHEN** `docker compose` exits with code 17
- **THEN** `cyamus compose` exits with code 17

### Requirement: Compose file selection
Without `-f`/`--file` in the arguments, `cyamus compose` SHALL use the files `docker compose` would select itself:
- **Base file.** Searching from the current directory upward to the filesystem root, the first directory containing a default compose file supplies it. When several are present, the first in this order is used: `compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml`.
- **Override file.** At most one override file from that same directory is added, the first of `compose.override.yml`, `compose.override.yaml`, `docker-compose.override.yml`, `docker-compose.override.yaml`.
- **`COMPOSE_FILE`.** When it is set, its files SHALL be used instead.
- **User `-f`.** When the user passes `-f`, exactly those files SHALL be used, in their order.

If no file is found, the command SHALL fail with an error naming the searched names.

#### Scenario: Default file with automatic override
- **WHEN** the worktree root contains `compose.yaml` and `compose.override.yaml`, and the user runs `cyamus compose up` with no `-f`
- **THEN** compose uses `compose.yaml`, then `compose.override.yaml`, then cyamus's override

#### Scenario: Run from a subdirectory
- **WHEN** `cyamus compose ps` runs in `<worktree>/src/app` and only the worktree root has `compose.yaml`
- **THEN** the root `compose.yaml` is used

#### Scenario: Preference among default names
- **WHEN** a directory contains both `docker-compose.yaml` and `docker-compose.yml`
- **THEN** `docker-compose.yml` is used, as `docker compose` does

#### Scenario: Explicit files
- **WHEN** the user runs `cyamus compose -f compose.dev.yaml up`
- **THEN** compose uses `compose.dev.yaml`, then cyamus's override, and no default files

### Requirement: Worktree stack environment
`cyamus compose` SHALL add to `docker compose`'s environment every variable `workspace setup` gives hooks, except `CYAMUS_EVENT`, computed the same way: the identity and path variables, `CYAMUS_DOMAIN`, `CYAMUS_PROXY_PORT`, `CYAMUS_URL_SUFFIX`, the manifest `[vars]` as `CYAMUS_VAR_*`, and the fingerprints as `CYAMUS_FINGERPRINT_*`. A variable already present in the environment SHALL be left as is. Runtime variables (`--var`) given to setup or teardown therefore apply when a hook runs `cyamus compose`, and a terminal invocation gets the manifest values. An invalid manifest SHALL make the command fail with the manifest error.

#### Scenario: Interpolation from a terminal
- **WHEN** a compose file sets `APP_URL: http://web.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}` and the user runs `cyamus compose up` in a terminal
- **THEN** the container receives the same `APP_URL` as when a setup hook runs it

#### Scenario: Manifest variable in a compose file
- **WHEN** the manifest sets `[vars] node-version = "22"` and a compose file uses `image: node:${CYAMUS_VAR_NODE_VERSION}`
- **THEN** `cyamus compose up` uses `node:22`

#### Scenario: Runtime variable from a hook
- **WHEN** setup runs with `--var node-version=24` and an `on_setup` hook runs `cyamus compose up -d`
- **THEN** compose sees `CYAMUS_VAR_NODE_VERSION=24`

### Requirement: Shared stack environment
`cyamus compose-shared`, and `cyamus compose` when it starts the shared stack, SHALL run the shared stack's `docker compose` with an environment that does not depend on the worktree it was run from:
- **Set to project-level values:**
  - `CYAMUS_PROJECT`, `CYAMUS_CONFIG_DIR`, `CYAMUS_ASSETS`, `CYAMUS_BIN` and `CYAMUS_CACHE_DIR`;
  - `CYAMUS_DOMAIN=<project>.localhost`, `CYAMUS_PROXY_PORT` and `CYAMUS_URL_SUFFIX`;
  - the manifest `[vars]` as `CYAMUS_VAR_*`, replacing inherited values.
- **Removed:** `CYAMUS_BRANCH`, `CYAMUS_BRANCH_SLUG`, `CYAMUS_BRANCH_SNAKE`, `CYAMUS_WORKSPACE`, `CYAMUS_EVENT`, every inherited `CYAMUS_VAR_*` not in the manifest, every `CYAMUS_FINGERPRINT_*`, and `COMPOSE_PROJECT_NAME`.

#### Scenario: Same configuration from any worktree
- **WHEN** the shared stack is started once by `cyamus compose up` in worktree `feat/x`, and once by `cyamus compose-shared up -d` in worktree `main`
- **THEN** both runs give compose the same environment, with no branch or workspace variables

#### Scenario: Shared service URL
- **WHEN** the shared compose file sets `MAILPIT_URL: http://mailpit.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}` and the port-80 relay is active
- **THEN** the value is `http://mailpit.myproj.localhost`

### Requirement: Service shadowing
A service whose name also exists in `<config>/compose.yaml` SHALL be shadowed in the worktree stack: `cyamus compose` SHALL NOT create or start it. Every other worktree service that lists it in `depends_on` SHALL keep its remaining dependencies, with their conditions, and drop only the shadowed ones. The user's compose files SHALL NOT be modified. When `<config>/compose.yaml` does not exist, nothing is shadowed.

#### Scenario: Shared database replaces the worktree one
- **WHEN** both the shared and the worktree compose files define `db`, and the worktree's `web` depends on `db` and `cache`
- **THEN** `cyamus compose up` starts `web` and `cache` but no worktree `db`, and `web` still waits for `cache`

#### Scenario: Nothing in common
- **WHEN** the two compose files have no service names in common
- **THEN** every worktree service is started as defined

### Requirement: Worktree network
When the project has a shared stack (`<config>/compose.yaml` exists), each worktree stack SHALL get its own Docker network `cyamus-<project>-<branch-label>`. Before running `docker compose`, `cyamus compose` SHALL create it if it is missing, with the labels `dev.cyamus.project=<project>` and `dev.cyamus.workspace=<branch-label>`. The generated override SHALL declare it external, and every worktree service SHALL join it in addition to the networks it already uses, except services with a `network_mode`. Each running container of the shared stack SHALL be connected to it, with its compose service name as an alias, before the worktree's containers are created. Worktree services therefore reach shared ones by service name. The network SHALL contain only that worktree's services and the shared ones, never another worktree's.

#### Scenario: Worktree reaches the shared database by name
- **WHEN** the shared stack runs `db`, and `db` is shadowed in a worktree stack whose `web` connects to `db:5432` at startup
- **THEN** the connection reaches the shared `db`

#### Scenario: Worktrees stay isolated
- **WHEN** worktrees `feat-a` and `feat-b` both run a service `api`, and `web` in `feat-a` resolves `api`
- **THEN** it always gets `feat-a`'s `api`, and names defined only in `feat-b` don't resolve

#### Scenario: Own networks are kept
- **WHEN** a worktree service declares `networks: [backend]`
- **THEN** it is attached to `backend` and to `cyamus-<project>-<branch-label>`, and not to the stack's `default` network

#### Scenario: Either stack can go down while the other runs
- **WHEN** a worktree stack is running and the user runs `cyamus compose-shared down`, or the shared stack is running and the user runs `cyamus compose down`
- **THEN** the command succeeds and the other stack keeps running

### Requirement: Worktree network cleanup
After a `cyamus compose down` that succeeded, cyamus SHALL disconnect the shared stack's containers from the worktree's network and remove the network. A failure here SHALL be reported as a warning without changing the exit code.

#### Scenario: Teardown removes the worktree network
- **WHEN** a teardown hook runs `cyamus compose down`
- **THEN** `cyamus-<project>-<branch-label>` no longer exists, and the shared containers keep running

### Requirement: Shared containers rejoin worktree networks
After every `cyamus compose-shared` invocation, and after `cyamus compose` brings the shared stack up, cyamus SHALL connect every running container of the shared stack to every existing network labelled `dev.cyamus.project=<project>` that it isn't already on, with its service name as an alias. Containers recreated by compose therefore stay reachable from running worktrees.

#### Scenario: Shared database recreated
- **WHEN** a worktree stack is running and the user runs `cyamus compose-shared up -d --force-recreate db`
- **THEN** the worktree's `web` can reach `db` again once the command returns

### Requirement: Starting the shared stack on demand
Before running `up` or `run` for a worktree stack, `cyamus compose` SHALL check whether `<config>/compose.yaml` exists and every one of its services has a running container. If not, it SHALL run the shared stack with `up -d` first. The user's arguments SHALL NOT be passed to the shared stack. If starting the shared stack fails, `cyamus compose` SHALL exit non-zero without running the worktree command. Other subcommands (`down`, `stop`, `restart`, `ps`, `logs`, and so on) SHALL NOT affect the shared stack. `workspace setup` SHALL NOT start the shared stack except through hooks that run `cyamus compose`.

#### Scenario: First worktree brings up shared services
- **WHEN** the shared stack is not running and a setup hook runs `cyamus compose up -d`
- **THEN** the shared stack is brought up detached and connected to the worktree's network, then the worktree stack is brought up

#### Scenario: Shared stack already running
- **WHEN** every shared service is running and the user runs `cyamus compose up -d`
- **THEN** the shared stack is not touched

#### Scenario: Worktree teardown leaves shared services up
- **WHEN** a teardown hook runs `cyamus compose down`
- **THEN** only the worktree stack and its network are removed

#### Scenario: Shared stack fails to start
- **WHEN** the shared stack's `up -d` fails
- **THEN** `cyamus compose up` exits non-zero and the worktree stack is not started

### Requirement: Compose version
Both commands SHALL require Docker Compose v2.24.4 or later, which supports the `!override` tag used in the generated override. With an older version, or without Compose, they SHALL fail with an error naming the required version.

#### Scenario: Old compose
- **WHEN** `docker compose version --short` reports `2.20.2`
- **THEN** the command fails and says Compose 2.24.4 or later is required
