# Spec Delta

## MODIFIED Requirements

### Requirement: Setup order of operations
Setup SHALL perform its steps in this order:
1. resolve the workspace and project;
2. load and validate the manifest;
3. apply assets;
4. compute fingerprints and ensure the cache directory exists;
5. register the workspace in the workspace registry;
6. ensure the daemon is running;
7. run `on_setup` hooks.

A failure at any step SHALL stop the remaining steps. The exceptions are a daemon that fails to start, and a workspace that can't be registered because of a label conflict or an empty label: both produce a warning and setup continues.

#### Scenario: Invalid manifest stops setup
- **WHEN** the manifest is invalid
- **THEN** no asset is applied, no hook runs, and the command exits non-zero

#### Scenario: Asset failure stops hooks
- **WHEN** an asset's source file is missing
- **THEN** no `on_setup` hook runs and the command exits non-zero

#### Scenario: Workspace registered before hooks
- **WHEN** an `on_setup` hook runs `docker compose up -d`
- **THEN** the workspace is already registered and the daemon already running when the containers start

#### Scenario: Daemon start failure does not block hooks
- **WHEN** the daemon cannot be started
- **THEN** setup warns, runs `on_setup` hooks, and its exit code depends only on the other steps

### Requirement: Teardown runs hooks and never deletes
`cyamus workspace teardown` SHALL load the manifest, compute fingerprints, ensure the cache directory exists, and run all matching `on_teardown` hook groups. It SHALL then unregister the workspace from the workspace registry. It SHALL unregister even if a hook failed, and the exit code SHALL still reflect the failure. It SHALL NOT delete the worktree directory, the branch, or any asset it applied.

#### Scenario: Teardown leaves the worktree in place
- **WHEN** teardown completes successfully
- **THEN** the worktree directory, its branch, and applied links and copies still exist, and the workspace is no longer registered

#### Scenario: Failed hook still unregisters
- **WHEN** an `on_teardown` hook exits with code 1
- **THEN** the workspace is unregistered and teardown exits non-zero

### Requirement: CYAMUS environment
Hooks SHALL run with the parent process environment plus:
- `CYAMUS_PROJECT`, `CYAMUS_BRANCH`, `CYAMUS_BRANCH_SLUG` (lowercase, non-alphanumeric runs to `-`) and `CYAMUS_BRANCH_SNAKE` (lowercase, non-alphanumeric runs to `_`);
- `CYAMUS_WORKSPACE` (worktree root), `CYAMUS_CONFIG_DIR`, `CYAMUS_ASSETS`, `CYAMUS_BIN`, `CYAMUS_CACHE_DIR` and `CYAMUS_EVENT`;
- `CYAMUS_VAR_*` and `CYAMUS_FINGERPRINT_*`;
- `CYAMUS_DOMAIN` (`<branch-label>.<project>.localhost`), `CYAMUS_PROXY_PORT` (the daemon port), and `CYAMUS_URL_SUFFIX`: empty when port 80 reaches the daemon (see the daemon spec), otherwise `:<daemon port>`;
- `COMPOSE_PROJECT_NAME` (`<project>-<branch-label>`), unless the parent environment already sets it.

#### Scenario: Environment for a branch
- **WHEN** an `on_setup` hook runs for project `myproj` on branch `feature/My-Thing` with the default daemon port
- **THEN** it sees `CYAMUS_PROJECT=myproj`, `CYAMUS_BRANCH=feature/My-Thing`, `CYAMUS_BRANCH_SLUG=feature-my-thing`, `CYAMUS_BRANCH_SNAKE=feature_my_thing`, `CYAMUS_EVENT=setup`, `CYAMUS_DOMAIN=feature-my-thing.myproj.localhost`, `CYAMUS_PROXY_PORT=1355`, `CYAMUS_URL_SUFFIX=:1355` (no redirect installed) and `COMPOSE_PROJECT_NAME=myproj-feature-my-thing`

#### Scenario: Event during teardown
- **WHEN** an `on_teardown` hook runs
- **THEN** it sees `CYAMUS_EVENT=teardown`

#### Scenario: URL suffix with the redirect
- **WHEN** port 80 reaches the daemon and an `on_setup` hook runs
- **THEN** it sees `CYAMUS_URL_SUFFIX` set to the empty string

#### Scenario: Caller's compose project name wins
- **WHEN** setup runs with `COMPOSE_PROJECT_NAME=custom` already in its environment
- **THEN** hooks see `COMPOSE_PROJECT_NAME=custom`
