# Proposal

## Why

Today Orca-created worktrees get their environment from git-workspace, a Python tool built around a bare-repo layout and its own worktree lifecycle (`up`/`rm`). Orca drives it through a workaround: its setup script calls `git-workspace reset`, and its teardown script calls `git-workspace rm`, which races Orca's own deletion. cyamus starts by porting the core of git-workspace to Rust, reshaped so Orca owns worktree creation and deletion and cyamus only sets up and tears down the environment of a worktree it is handed.

## What Changes

- New Rust CLI binary `cyamus`, standalone (not a git subcommand).
- **Projects**: per-repository configuration identified by the git config key `cyamus.project` in the git common dir, so bare and regular checkouts behave the same. If the key is unset, the first setup guesses it from the `origin` remote's repo name, persists it and reports it.
- **Config lives outside the repo** at `$XDG_CONFIG_HOME/cyamus/projects/<project>/` (`cyamus.toml`, `assets/`, `bin/`), with XDG paths used on macOS too.
- **Manifest** `cyamus.toml`: `[vars]`, `[[link]]`, `[[copy]]`, `[[fingerprint]]`, `[[hooks.on_setup]]`, `[[hooks.on_teardown]]`, with branch-glob `conditions` on hook groups.
- `cyamus workspace setup [path]`: idempotent. It applies links and copies (including override mode and `info/exclude` handling), computes fingerprints and runs `on_setup` hooks. Orca's setup script calls it, and it also serves as "reset".
- `cyamus workspace teardown [path]`: runs `on_teardown` hooks. It never deletes the worktree directory or branch, because Orca does that.
- `cyamus edit`: opens `$EDITOR` on the project's config directory.
- `CYAMUS_*` environment variables for hooks, replacing `GIT_WORKSPACE_*`.
- **Fingerprints**: SHA-256 hashes over listed worktree files, exposed as `CYAMUS_FINGERPRINT_*`, so hooks can skip unchanged work such as `npm install` or `docker build`.
- **Project cache directory** `$XDG_CACHE_HOME/cyamus/projects/<project>/`, exposed as `CYAMUS_CACHE_DIR`. It is shared by a project's workspaces and outlives worktrees, so hooks can store previous fingerprints there. It replaces git-workspace's `cache` command.
- **BREAKING** (relative to git-workspace, no compatibility kept): no `init`/`clone`/`up`/`down`/`rm`/`ls`/`prune`, no `on_attach`/`on_detach`, no `base_branch`, no `[prune]`, no `cache` command, fingerprints are SHA-256 only (no `algorithm` key), no in-repo `.workspace/` config directory, new env var prefix, new manifest file name and location.
- Not in this change, planned for later ones: Jinja templating replacement (`templating`), `doctor` and `compose` (`port-rest`), `[[service]]`/ports/registry (`daemon-registry`), reverse proxy (`daemon-proxy`). Until `templating` lands, copies are byte-for-byte.

## Capabilities

### New Capabilities

- `project-config`: identifying a checkout's project (git config key, guess-and-persist), locating its config directory under XDG paths, and opening it with `cyamus edit`.
- `manifest`: the `cyamus.toml` schema (vars, links, copies, fingerprints, hook groups and conditions), parsing and validation errors.
- `workspace-lifecycle`: the `cyamus workspace setup` / `teardown` commands, resolving a workspace from a path, the order of operations, the `CYAMUS_*` environment, exit codes, and the "never delete" guarantee.
- `workspace-assets`: applying `[[link]]` and `[[copy]]` entries into a worktree, including overwrite control, override mode (`skip-worktree`) and git exclusion of targets.
- `workspace-fingerprints`: computing file-set fingerprints, exposing them to hooks, and the per-project cache directory where hooks keep state across worktrees.
- `workspace-hooks`: resolving and running `on_setup` / `on_teardown` hook commands (bin scripts vs inline), branch-glob conditions, working directory, environment and failure behaviour.

### Modified Capabilities

None. This is the first change and `openspec/specs/` is empty.

## Impact

- New Cargo project at the repo root: a single binary, internally split into modules or crates so that the later daemon work slots in cleanly.
- Expected dependencies: `clap` (CLI), `serde` + `toml` (manifest), manual XDG resolution, `sha2` (fingerprints), `globset` or equivalent (branch conditions), `thiserror`/`anyhow` (errors). Templating and Docker crates come in later changes.
- Requires `git` on `PATH`. Shells out for `rev-parse`, `config`, `update-index`.
- Orca lifecycle scripts change from `git-workspace reset` / `git-workspace rm` to `cyamus workspace setup` / `cyamus workspace teardown`.
- Existing git-workspace configs must be migrated by hand: move `.workspace/` contents into `~/.config/cyamus/projects/<project>/` and rename env vars.
