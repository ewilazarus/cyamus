# cyamus

An opinionated worktree environment lifecycle for the [Orca ADE](https://www.onorca.dev/).

Orca owns the UI and creates and deletes git worktrees. cyamus sets up each worktree's environment when Orca creates it, and tears it down when Orca closes it. It links and copies your local config files into the worktree, runs your setup and teardown hooks, and gives hooks the context they need.

> *Cyamus orcini* is a whale louse that lives on orcas.

## Vocabulary

- **Project**: per-repository configuration shared by all its worktrees. It lives outside the repository, under `~/.config/cyamus/projects/<project>/`.
- **Workspace**: one Orca-created worktree with cyamus's assets and hooks applied to it.

## Installation

```sh
curl -fsSL https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh | sh
```

The script downloads the prebuilt binary for your platform, checks its SHA-256, and installs it as `~/.local/bin/cyamus`. It never uses `sudo` and never edits your shell profile. If the install directory isn't on your `PATH`, it prints the line to add. Run it again to upgrade.

| Variable | Effect |
|---|---|
| `CYAMUS_VERSION` | Install a specific release instead of the latest, e.g. `0.2.0` or `v0.2.0` |
| `CYAMUS_INSTALL_DIR` | Install into this directory instead of `~/.local/bin` |

```sh
curl -fsSL https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh | CYAMUS_VERSION=0.2.0 sh
```

To read the script before running it:

```sh
curl -fsSLO https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh
less install.sh
sh install.sh
```

### From source

On platforms without a prebuilt binary, install with cargo (Rust 1.89+):

```sh
cargo install --git https://github.com/ewilazarus/cyamus cyamus-cli
# or, from a checkout:
cargo install --path crates/cyamus-cli
```

Developing on cyamus itself uses [devbox](https://www.jetify.com/devbox) for the toolchain:

```sh
devbox run check   # fmt + clippy + tests
```

## Orca setup

Point Orca's worktree lifecycle scripts at cyamus:

| Orca script | Command |
|---|---|
| Setup | `cyamus workspace setup` |
| Teardown | `cyamus workspace teardown` |

Both commands act on the worktree containing the current directory, or on the path you pass (`cyamus workspace setup /path/to/worktree`). Neither command creates or deletes worktrees or branches; Orca does that.

## Projects

A checkout's project name is stored in git config as `cyamus.project`, which every worktree of the repository shares. This works the same for bare and regular checkouts. If the key isn't set, the first cyamus command guesses a name from the `origin` remote (or from the repository directory), saves it, and tells you:

```
cyamus: guessed project name "my-proj" and saved it as `cyamus.project` in git config
```

To choose the name yourself, run `git config cyamus.project <name>`. Names must be valid DNS labels: lowercase letters, digits and hyphens, at most 63 characters.

### Config directory

```
$XDG_CONFIG_HOME/cyamus/projects/<project>/     (default: ~/.config/cyamus/projects/<project>/)
├── cyamus.toml   ← manifest
├── assets/       ← files linked or copied into each worktree
└── bin/          ← hook scripts
```

cyamus uses XDG paths on every platform, including macOS. Run `cyamus edit` from anywhere inside the repository to open this directory in `$EDITOR`. If it doesn't exist yet, cyamus creates it with an empty `assets/`, an empty `bin/` and a minimal `cyamus.toml`. `EDITOR` may include arguments, e.g. `EDITOR="code -w"`.

Until the config directory exists, `setup` and `teardown` print a notice and exit successfully. This means you can install the Orca scripts before configuring a project.

## Commands

| Command | What it does |
|---|---|
| `cyamus workspace setup [path] [--var K=V]...` | Applies assets, computes fingerprints, runs `on_setup` hooks. Idempotent; re-run it to reset a workspace. |
| `cyamus workspace teardown [path] [--var K=V]...` | Computes fingerprints, runs `on_teardown` hooks. Never deletes anything. |
| `cyamus edit [path]` | Opens the project config directory in `$EDITOR`. |

## Manifest

```toml
version = 1

# Exposed to hooks as CYAMUS_VAR_<NAME> (uppercased, non-alphanumerics → _).
[vars]
node-version = "22"

# Symlink assets/env.local → <worktree>/.env.local
[[link]]
source = "env.local"
target = ".env.local"

# Copy assets/config.local.yaml → <worktree>/config.local.yaml
[[copy]]
source = "config.local.yaml"
target = "config.local.yaml"
overwrite = false        # seed once, keep local edits (default: true)

# Replace a tracked file without touching git history.
[[link]]
source = "vscode-settings.json"
target = ".vscode/settings.json"
override = true

# Exposed to hooks as CYAMUS_FINGERPRINT_DOCKER_DEPS.
[[fingerprint]]
name = "docker-deps"
files = ["Dockerfile", "package-lock.json"]
length = 12              # optional, 1-64 (default: 12)

[[hooks.on_setup]]
commands = ["install_deps", "npm run build"]

[[hooks.on_setup]]
conditions = { if_branch_matches = "gabriel/*", if_branch_not_matches = "gabriel/wip-*" }
commands = ["echo my branch"]

[[hooks.on_teardown]]
commands = ["docker compose down"]
```

Unknown keys are errors, so typos are caught early. Errors include the manifest path and, for syntax errors, the line and column.

### Assets

- `source` is relative to `assets/` and `target` to the worktree root. Neither may escape its directory.
- **Links** point to the absolute path of the source. An existing link to the same source is left alone. Any other existing file at the target is an error.
- **Copies** are byte-for-byte, and directories are copied recursively with permissions preserved. With `overwrite = true` (the default), every setup replaces the target. With `overwrite = false`, an existing target is never touched.
- Targets are hidden from `git status` by a cyamus-managed block in the repository's shared `info/exclude`. Your own lines in that file are preserved.
- **`override = true`** replaces a *tracked* file. The file is marked `skip-worktree` so git ignores the change. Targeting an untracked path with `override` is an error, and so is targeting a tracked path without it.

### Hooks

Each entry in `commands` runs as `bin/<entry>` if that file exists, and must then be executable. Otherwise it runs as an inline command through `$SHELL -c` (falling back to `sh`). Hooks run one at a time in the worktree root, with output streamed to your terminal. The first failure stops the run, and cyamus exits non-zero.

A group runs when it has no `conditions`, or when the branch matches `if_branch_matches` **and** doesn't match `if_branch_not_matches`. In patterns, `*` matches anything including `/`, `?` matches one character, and `[...]` matches a character class.

### Environment

Hooks inherit cyamus's environment, plus:

| Variable | Value |
|---|---|
| `CYAMUS_PROJECT` | Project name |
| `CYAMUS_BRANCH` | Branch name (e.g. `feature/My-Thing`) |
| `CYAMUS_BRANCH_SLUG` | `feature-my-thing` |
| `CYAMUS_BRANCH_SNAKE` | `feature_my_thing` |
| `CYAMUS_WORKSPACE` | Worktree root |
| `CYAMUS_CONFIG_DIR` | Project config directory |
| `CYAMUS_ASSETS` | `<config>/assets` |
| `CYAMUS_BIN` | `<config>/bin` |
| `CYAMUS_CACHE_DIR` | Per-project cache directory (see below) |
| `CYAMUS_EVENT` | `setup` or `teardown` |
| `CYAMUS_VAR_*` | Manifest `[vars]`, overridden by `--var KEY=VALUE` |
| `CYAMUS_FINGERPRINT_*` | Fingerprint digests |

### Fingerprints and the cache directory

A fingerprint is a short SHA-256 digest over a list of worktree files. It's computed after assets are applied, so copied files are included. A missing file counts as a marker instead of failing.

`CYAMUS_CACHE_DIR` (`$XDG_CACHE_HOME/cyamus/projects/<project>/`, default `~/.cache/...`) is shared by every workspace of the project and outlives worktrees. cyamus creates it but never cleans it. Together they let hooks skip work whose inputs haven't changed:

```sh
#!/bin/sh
# bin/build_image
state="$CYAMUS_CACHE_DIR/docker-deps"
if [ "$CYAMUS_FINGERPRINT_DOCKER_DEPS" != "$(cat "$state" 2>/dev/null)" ]; then
    docker build . -t myapp
    echo "$CYAMUS_FINGERPRINT_DOCKER_DEPS" > "$state"
fi
```

## Platform support

Developed and verified on macOS. Linux is expected to work. Windows is not supported.

Prebuilt binaries are published for:

| Platform | Target |
|---|---|
| macOS, Apple Silicon | `aarch64-apple-darwin` |
| Linux, x86_64 | `x86_64-unknown-linux-musl` (static) |
| Linux, arm64 | `aarch64-unknown-linux-musl` (static) |

Intel Macs are supported [from source](#from-source) only.

## Releasing

1. Bump `version` under `[workspace.package]` in `Cargo.toml` and commit.
2. Tag the commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The [release workflow](.github/workflows/release.yml) checks that the tag matches the crate version and runs lint and tests. It then builds every target and publishes a GitHub Release with the archives, their `.sha256` files and `install.sh`. If any step fails, nothing is published.
