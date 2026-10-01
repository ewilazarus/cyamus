# Design

## Context

The repository is empty apart from OpenSpec scaffolding. The behaviour being ported comes from git-workspace (Python, about 4.4k lines). Its relevant modules are `manifest.py`, `assets.py`, `hooks.py`, `env.py` and `workspace.py`. Two things change in the port: Orca, not cyamus, creates and deletes worktrees, and configuration moves out of the repository into XDG directories (see proposal.md and `openspec/config.yaml`). Later changes will add templating, fingerprints, doctor, compose, a port registry and a long-running daemon to the same binary, so the structure chosen here has to accommodate them.

## Goals / Non-Goals

**Goals:**
- A code layout where the synchronous CLI core never depends on async, network or Docker crates, so the daemon can be added later as a separate component of the same binary.
- Behaviour that is testable end to end against real git repositories, without mocks, as git-workspace's integration suite does.
- Safe concurrent use: Orca may create several worktrees of the same repository at once, which runs several `setup` processes in parallel.

**Non-Goals:**
- Rich or styled terminal UI (progress bars, tables). Output is plain text. Hook output passes through untouched.
- A library API for third parties. Crate boundaries exist for internal structure only.
- Branch impersonation (`--as`) and session hooks. Orca has no use for them today.

## Decisions

### Cargo workspace: core library + thin CLI
```
Cargo.toml                (workspace)
crates/
  cyamus-core/            project, paths, git, manifest, env, assets, hooks, lifecycle
  cyamus-cli/             clap definitions, output, exit codes → binary `cyamus`
  (later) cyamus-registry/, cyamus-daemon/  linked into the same binary
```
A workspace enforces which crate may depend on which. `cyamus-core` cannot accidentally pull in tokio or axum, and the daemon crate can depend on core types without the reverse ever happening. *Alternative:* a single crate with modules. It's simpler today, but nothing would stop the boundaries from eroding, and the user asked for the code to be well split.

### Shell out to the `git` CLI
All git interaction (`rev-parse --show-toplevel/--git-common-dir`, `symbolic-ref --short HEAD`, `config`, `remote get-url`, `ls-files --error-unmatch`, `update-index --skip-worktree`) goes through one small `Git` type in core that runs `git -C <path>`. *Alternatives:* `git2` needs libgit2 linking and has partial worktree semantics. `gix` is pure Rust, but its worktree and config-writing support is still evolving. The git CLI is the reference behaviour for worktrees and config precedence, and git-workspace already relied on it. `git config --local` run inside a linked worktree writes to the common dir's config, which is exactly where `cyamus.project` belongs.

### Manual XDG resolution
`XDG_CONFIG_HOME` falls back to `$HOME/.config`, and `XDG_CACHE_HOME` to `$HOME/.cache`. `XDG_STATE_HOME` falls back to `$HOME/.local/state` and is used by later changes. This is a few lines in a `paths` module. *Alternatives:* `dirs` maps to `~/Library/Application Support` on macOS, which contradicts the decision. `etcetera` would work, but adds a dependency for about 15 lines of code.

### Manifest: serde with strict schema, validation as a second pass
`serde` + `toml` with `#[serde(deny_unknown_fields)]` on every struct produces "unknown key" errors with spans, which covers the typo and removed-key requirements. Semantic checks that serde can't express (variable-name collisions, unsupported version, path escapes) run as a separate validation pass that returns all errors at once. The later `doctor` change will reuse this pass.

### Branch conditions via `globset`
`globset` with the default `literal_separator(false)` gives fnmatch semantics where `*` crosses `/`, matching git-workspace and the spec. *Alternative:* `glob::Pattern` would also work, but `globset` is the more maintained of the two and is already the common choice.

### Hooks inherit stdio, attributed with a header line
Each hook runs through `std::process::Command` with inherited stdout and stderr. Before each command, cyamus writes a header such as `▸ on_setup: npm install` to stderr. Inheriting keeps the hook's TTY detection, colours and progress output intact and streams in real time without reader threads. *Alternative:* pipe the output and prefix every line, as git-workspace's Rich UI did. That breaks TTY-aware tools and needs per-stream threads, for marginal benefit.

### Bin scripts are executed directly, inline commands via `$SHELL -c`
This keeps git-workspace semantics. Bin scripts get a clear "not executable" error instead of a shell's `permission denied`.

### `info/exclude` managed block: anchored paths, locked, atomic
The exclude file lives in the git common dir and is shared by every worktree. The block is regenerated from the manifest's non-override targets as root-anchored patterns (`/path/to/target`). All worktrees of a project share the manifest, so every setup writes the same block. Writes happen under an exclusive lock (`std::fs::File::lock`, stable since Rust 1.89) and go through write-temp-then-rename, so parallel setups from Orca can't interleave or truncate the file. This fixes a latent issue in git-workspace, which wrote absolute paths with no locking.

### Project-name guessing
The candidate comes from the last path segment of `git remote get-url origin` with `.git` stripped, or else from the common dir's name (`.git` → parent folder name, `foo.git` → `foo`). It is lowercased, every run of non-`[a-z0-9]` becomes `-`, leading and trailing `-` are trimmed, and the result is truncated to 63 characters. It is persisted with `git config --local cyamus.project <name>` and reported on stderr. If the write fails because a parallel setup holds git's config lock, cyamus reads the key again and uses whatever the other process wrote. The same rule produces `CYAMUS_BRANCH_SLUG`, so hostnames later use identical slugging.

### Fingerprints: SHA-256 only, computed after assets
The hashing format follows git-workspace: files sorted, `path bytes + content bytes` (or `NULL` for a missing file) fed into one hasher, the hex digest truncated. It uses the `sha2` crate. The `algorithm` key and MD5 support are dropped: with no backward compatibility to keep, a second algorithm adds a config option and a doctor check for no benefit. git-workspace computed fingerprints before applying assets. cyamus computes them *after*, so a fingerprint over a copied file sees the copy on the very first setup instead of `NULL`.

### Cache directory instead of a `cache` command
`CYAMUS_CACHE_DIR` (`$XDG_CACHE_HOME/cyamus/projects/<project>/`, created on demand) replaces git-workspace's `cache get/set` command and its per-script namespaces. Hooks are shell scripts, and shell already handles files well. A plain directory covers the main use, storing a previous fingerprint, without a subcommand, key validation or namespacing rules. It goes in XDG *cache* rather than *state* because everything in it can be regenerated, and users may wipe `~/.cache` freely. *Alternative:* port `cache get/set`. This can be added later on top of the same directory if hook scripts keep repeating the same snippet.

### Errors and exit codes
`thiserror` enums in core, one per module, wrapped in a top-level error. The CLI renders the error chain to stderr and exits 1. Hook failures keep the hook name and its exit code in the error so the message identifies them. The "unconfigured project" case is a notice, not an error, and exits 0 (see the workspace-lifecycle spec).

### Testing strategy
- Unit tests in core: slug normalization, variable collisions, glob matching, path-escape checks, exclude-block rewriting, fingerprint digests (golden values).
- Integration tests in `cyamus-cli/tests/` using `assert_cmd` and `tempfile`. Each test creates a real regular *and* bare repository, adds linked worktrees with `git worktree add` (imitating Orca), points `XDG_CONFIG_HOME` at a temp directory, and runs the binary.

## Risks / Trade-offs

- [Orca's reaction to a failing teardown script is unknown. It might refuse to delete the worktree.] → The spec makes hook failures exit non-zero, which is honest. If this turns out to block deletion, a later change can add `--keep-going` or make teardown exit 0 with a warning. That would be a small spec change.
- [Two unrelated repositories with the same name guess the same project and silently share config.] → The guess is always reported. Users can set `cyamus.project` explicitly. A future `doctor` check can flag it.
- [Links point to absolute paths under `~/.config`. Moving the config directory breaks existing links.] → Running `setup` again repairs them. Documented.
- [Rejecting unknown keys can make a manifest written for a newer cyamus fail on an older build.] → `version` exists for this, and strictness catches far more typos than it causes compatibility problems for a single-user tool.
- [Inherited stdio means interleaved output can't be attributed line by line.] → Hooks run sequentially, so each hook's output is bracketed by its header.

## Migration Plan

1. Install cyamus alongside git-workspace.
2. For each project: `git config cyamus.project <name>` (or let the first setup guess it), then move `.workspace/{manifest.toml→cyamus.toml, assets/, bin/}` into `~/.config/cyamus/projects/<name>/`. Rename `GIT_WORKSPACE_*` references in scripts to `CYAMUS_*` (`GIT_WORKSPACE_WORKTREE` becomes `CYAMUS_WORKSPACE`), and drop removed keys, including fingerprint `algorithm`. Scripts that kept fingerprint state under `$GIT_WORKSPACE_ROOT` or used `git workspace cache` switch to files under `$CYAMUS_CACHE_DIR`.
3. Change the Orca setup and teardown scripts to `cyamus workspace setup` / `cyamus workspace teardown`.
4. Rollback: point the Orca scripts back at git-workspace. cyamus leaves no state behind other than the `cyamus.project` git config key and its `info/exclude` block.
