# workspace-lifecycle Specification

## Purpose
Defines the two commands Orca's lifecycle scripts call, `cyamus workspace setup` and `cyamus workspace teardown`: how they resolve a workspace, the order in which they act, the environment they expose and the guarantees they keep.

## Requirements

### Requirement: Workspace resolution from a path
`cyamus workspace setup [path]` and `cyamus workspace teardown [path]` SHALL resolve the workspace from `path` (default: the current directory). The path may be the worktree root or any directory inside it. The worktree root, current branch, git common dir and project SHALL be determined from git.

#### Scenario: Run from a subdirectory
- **WHEN** setup runs with the current directory at `<worktree>/src/lib`
- **THEN** the workspace resolves to `<worktree>` and its current branch

#### Scenario: Explicit path
- **WHEN** Orca runs `cyamus workspace setup /path/to/worktree` from another directory
- **THEN** the workspace resolves to `/path/to/worktree`

#### Scenario: Not a git worktree
- **WHEN** the path is not inside a git worktree
- **THEN** the command fails with a non-zero exit code and an error saying so

#### Scenario: Detached HEAD
- **WHEN** the worktree's HEAD is detached
- **THEN** the command fails with a non-zero exit code and an error saying a branch is required

### Requirement: Setup is idempotent and serves as reset
`cyamus workspace setup` SHALL apply all assets and then run all matching `on_setup` hook groups every time it is invoked. Running it repeatedly on the same workspace SHALL be safe and SHALL reapply the configuration.

#### Scenario: First setup
- **WHEN** setup runs on a freshly created worktree
- **THEN** assets are applied, then `on_setup` hooks run

#### Scenario: Repeated setup
- **WHEN** setup runs again on the same worktree
- **THEN** assets are reapplied according to their overwrite rules and `on_setup` hooks run again

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

### Requirement: Missing project configuration is not an error
If the project's config directory does not exist, setup and teardown SHALL print a notice that names the expected path and SHALL exit with code 0 without applying assets or running hooks. This lets Orca scripts be installed before a project is configured.

#### Scenario: Unconfigured project
- **WHEN** setup runs for project `myproj` and `~/.config/cyamus/projects/myproj/` does not exist
- **THEN** a notice names that path, nothing is applied, and the exit code is 0

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

### Requirement: Runtime variables
Setup and teardown SHALL accept repeatable `--var KEY=VALUE` options. Runtime variables SHALL be exposed as `CYAMUS_VAR_*` and SHALL take precedence over manifest `[vars]` with the same normalized name.

#### Scenario: Runtime overrides manifest
- **WHEN** the manifest sets `env = "dev"` and setup runs with `--var env=staging`
- **THEN** hooks see `CYAMUS_VAR_ENV=staging`

#### Scenario: Malformed runtime variable
- **WHEN** setup runs with `--var novalue`
- **THEN** the command fails with an error before resolving anything

### Requirement: Exit codes
Setup and teardown SHALL exit with code 0 on success and a non-zero code on any failure, writing a human-readable error to stderr.

#### Scenario: Hook failure propagates
- **WHEN** an `on_teardown` hook exits with code 3
- **THEN** teardown exits non-zero and stderr names the failing hook and its exit code
