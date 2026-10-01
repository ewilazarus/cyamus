# Proposal

## Why

Most projects run some services once per machine (Postgres, Redis, Mailpit) and the rest once per worktree. In git-workspace this took three hand edits:
- a manual `docker network create`;
- a `networks:` block in both compose files;
- an "omitted" profile in the copied compose file, so worktrees wouldn't start duplicate databases.

These edits also broke `depends_on`. All of it encodes one fact cyamus can work out on its own: a service defined in the project's shared compose file comes from the shared stack.

## What Changes

- **`cyamus compose-shared [args…]`** runs `docker compose` for the project's shared stack: `<config>/compose.yaml`, with compose project name `<project>`. Arguments pass through, and compose's exit code is preserved. Its environment is project-level only: project, paths, manifest `[vars]`, and `CYAMUS_DOMAIN=<project>.localhost`. Worktree variables are stripped, so the shared stack is configured the same no matter which worktree started it.
- **`cyamus compose [args…]`** runs `docker compose` for the current worktree's stack:
  - Compose project name `<project>-<branch-label>`.
  - The same files `docker compose` would pick on its own, or the user's `-f` files.
  - The same `CYAMUS_*` variables hooks get, including manifest `[vars]` and fingerprints, filled in only where missing. `${…}` interpolation then works the same from hooks and from a terminal, and a hook's runtime `--var` overrides still apply.
- **Service shadowing.** A service that also exists in the shared compose file is left out of the worktree stack. A generated override file gives it an inactive profile and rewrites its dependents' `depends_on` without it (keeping their other dependencies). The repository's compose files are never edited.
- **A network per worktree.** cyamus creates `cyamus-<project>-<branch-label>`, external to the worktree stack. Every worktree service joins it alongside its own networks, and the shared containers are connected to it under their service names before the worktree starts. Worktree services reach shared ones by name (`db:5432`), and worktrees never see each other. `cyamus compose down` removes the network. After every `cyamus compose-shared` call, the shared containers are reattached to all of the project's worktree networks.
- **`cyamus compose up` and `run` start the shared stack first** when it is defined and not fully running (`up -d`). If that fails, the command stops. Other subcommands never touch the shared stack. `workspace setup` never starts it on its own.
- **Routing.** HTTP services of the shared stack get `http://<service>.<project>.localhost/`. The hostname shape reserved by the proxy-routing spec is now produced.
- **README.** Documents both commands and shadowing, and gives a worked example: one shared Postgres with a database per worktree, created and dropped by hooks using `CYAMUS_BRANCH_SNAKE`.

## Capabilities

### New Capabilities
- `compose-stacks`: the `cyamus compose` and `cyamus compose-shared` commands. Covers compose project naming, file selection, the environment passed to compose, service shadowing, the cyamus-owned shared network, and starting the shared stack on `up`/`run`.

### Modified Capabilities
- `docker-discovery`: containers whose compose `working_dir` is a project's config directory belong to that project's shared stack and become routes.
- `proxy-routing`: `<service>.<project>.localhost` hostnames are now produced, for shared-stack services.

## Impact

- **Code:**
  - New `compose` module in `cyamus-core`. It is synchronous and shells out to the `docker` CLI; core gains no Docker crate.
  - Two new CLI subcommands.
  - Project-scoped membership and routes in `cyamus-daemon`.
- **State:** generated override files under `$XDG_STATE_HOME/cyamus/compose/`, rewritten on every invocation.
- **Requirements:** Docker Compose v2.24.4 or later (for the `!override` YAML tag). Older versions get a clear error.
- **Docker:** one network per running worktree stack (`cyamus-<project>-<label>`, labelled), removed by `cyamus compose down`.
- **Hooks:** the README's Docker example switches from `docker compose up -d` to `cyamus compose up -d`.
