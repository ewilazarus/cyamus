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
| `cyamus workspace setup [path] [--var K=V]...` | Applies assets, computes fingerprints, registers the workspace, starts the [routing daemon](#daemon-and-routing) if needed, and runs `on_setup` hooks. Idempotent; re-run it to reset a workspace. |
| `cyamus workspace teardown [path] [--var K=V]...` | Computes fingerprints, runs `on_teardown` hooks, unregisters the workspace. Never deletes anything in the worktree. |
| `cyamus daemon status` | Shows the routing daemon, its routes, and containers it isn't routing (with the reason). Exits 1 when the daemon isn't running. |
| `cyamus daemon stop` | Stops the routing daemon. |
| `cyamus daemon install [--dry-run]` | One-time, uses `sudo`: redirects port 80 to the daemon (this machine only), so URLs need no port. `uninstall` removes it. |
| `cyamus daemon run` | Runs the routing daemon in the foreground. You rarely need this, because setup starts it for you. |
| `cyamus compose [args…]` | `docker compose` for this worktree's stack, with shared services left out and reachable by name ([Docker](#docker)). |
| `cyamus compose-shared [args…]` | `docker compose` for the project's shared stack in `<config>/compose.yaml`. |
| `cyamus edit [path]` | Opens the project config directory in `$EDITOR`. |

## Agent skill

[`SKILL.md`](SKILL.md) teaches coding agents working in a cyamus worktree how to use it. It covers using `cyamus compose` rather than `docker compose`, finding a service's URL, and what not to touch: the shared stack, teardown, and linked files. Install it with:

```sh
npx skills add https://github.com/ewilazarus/cyamus --skill cyamus --global
```

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
| `CYAMUS_DOMAIN` | `feature-my-thing.my-proj.localhost`: append it to a service name to get its [hostname](#daemon-and-routing) |
| `CYAMUS_PROXY_PORT` | Port the routing daemon listens on (default `1355`) |
| `CYAMUS_URL_SUFFIX` | What goes after a hostname in a URL: empty once [`cyamus daemon install`](#urls-without-a-port) is in place, `:1355` before. `http://web.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}` is always right. |
| `PATH` | Your `PATH`, with the directory of the running `cyamus` first, so hooks can call `cyamus` even when Orca was started from the Dock |
| `COMPOSE_PROJECT_NAME` | `my-proj-feature-my-thing`, so each worktree gets its own compose stack. Not set if your environment already sets it. |

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

## Daemon and routing

Every service of a workspace gets a stable URL:

```
http://<service>.<branch>.<project>.localhost/
```

For example, `http://web.feature-my-thing.my-proj.localhost/`. `<branch>` is `CYAMUS_BRANCH_SLUG`, cut to 63 characters. Browsers, curl and most runtimes resolve `*.localhost` to your machine without any DNS setup, so the URL works from host processes too.

A small routing daemon serves these URLs. It's built into the `cyamus` binary:

- `cyamus workspace setup` starts it in the background if it isn't running, and replaces it after you upgrade cyamus. Set `CYAMUS_DAEMON=off` to keep setup from starting it.
- It listens only on this machine (`127.0.0.1` and `::1`), on port `1355`, so nothing on your network can reach it. Set `CYAMUS_DAEMON_PORT` to use another port. Setup and the daemon must agree on it, so export it from your shell profile.
- Run `cyamus daemon install` once to drop the port from URLs (see below). Until then, URLs carry `:1355`, e.g. `http://web.feature-my-thing.my-proj.localhost:1355/`.
- It runs until `cyamus daemon stop`, logging to `$XDG_STATE_HOME/cyamus/daemon.log` (default `~/.local/state/...`).
- [`http://cyamus.localhost/`](http://cyamus.localhost/) (or `http://cyamus.localhost:1355/`) lists every route, as does `cyamus daemon status`.
- It forwards plain HTTP, including WebSockets and server-sent events, so dev-server hot reload works. It keeps the original `Host` header and adds `X-Forwarded-Host`, `X-Forwarded-Proto` and `X-Forwarded-For`.
- An unknown hostname gets a 404 that lists the routes. A route whose service isn't answering yet gets a 502.

HTTPS isn't supported yet. Setup registers each workspace in `$XDG_STATE_HOME/cyamus/workspaces/`, and teardown removes it. That registry is how the daemon knows which worktrees exist.

### URLs without a port

The daemon itself never listens on port 80. Binding port 80 would mean listening on every network interface, and on macOS that triggers a firewall prompt. Instead, a one-time install adds a tiny **relay**: a system service that listens on `127.0.0.1:80` and `[::1]:80` only, and passes each connection through to the daemon unchanged.

```sh
cyamus daemon install            # asks for your sudo password
cyamus daemon install --dry-run  # shows exactly what it would change
cyamus daemon uninstall          # undoes it
```

Run it without `sudo`: it prints each command before running it with `sudo`, then checks that `http://cyamus.localhost/` reaches the daemon.

| | What install changes |
|---|---|
| Both | `/usr/local/libexec/cyamus-relay`: a root-owned copy of `cyamus` that the service runs. It never runs your own binary, which anything running as you could replace. |
| macOS | `/Library/LaunchDaemons/dev.cyamus.relay.plist`: starts the relay at boot and restarts it if it exits. Binding port 80 needs root, so the relay binds as root, then handles connections from a child process running as `nobody`. Log: `/var/log/cyamus-relay.log`. |
| Linux | `/etc/systemd/system/cyamus-relay.service`: runs the relay as an ephemeral unprivileged user (`DynamicUser=yes`) with only `CAP_NET_BIND_SERVICE`. Requires systemd. Log: `journalctl -u cyamus-relay`. |

The relay forwards to the daemon port in use at install time: if you change `CYAMUS_DAEMON_PORT`, run install again. `cyamus daemon status` tells you whether port 80 reaches the running daemon.

## Docker

Containers become routes without any cyamus-specific configuration. When compose runs inside a worktree, the daemon matches the container's compose working directory against registered worktrees, and names the route after the compose service.

Run compose through cyamus, from hooks and from your terminal:

| | Stack | Compose project | Files |
|---|---|---|---|
| `cyamus compose …` | this worktree's services | `<project>-<branch>` | what `docker compose` would pick here (`compose.yaml` and its override, searching upward), or your `-f` files |
| `cyamus compose-shared …` | services shared by every worktree | `<project>` | `<config>/compose.yaml` |

Arguments are passed through unchanged, and so is the exit code. Both commands give compose the same `CYAMUS_*` variables hooks get, so `${CYAMUS_DOMAIN}` and friends interpolate the same everywhere. Docker Compose 2.24.4 or later is required.

### Shared services

Put services that every worktree can share (a database, a cache, a mail catcher) in `<config>/compose.yaml`. When a worktree's compose file defines a service **with the same name**, `cyamus compose` uses the shared one instead:

- **No duplicates.** The worktree's own `db` isn't started. Services that depended on it keep their other dependencies.
- **Reachable by name.** `web` reaches the shared service at `db:5432`, as if it were in its own stack. Each worktree gets a private network (`cyamus-<project>-<branch>`) that only it and the shared services join, so worktrees never see each other.
- **Started on demand.** `cyamus compose up` and `cyamus compose run` start the shared stack first if it isn't fully running. It keeps running when worktrees are torn down; stop it with `cyamus compose-shared down`.
- **Visible.** `cyamus compose` prints the services it takes from the shared stack (`using shared: db`).
- **Routed.** Shared HTTP services get `http://<service>.<project>.localhost/`. In the shared stack, `CYAMUS_DOMAIN` is `<project>.localhost`.
- **Untouched files.** Your compose files are never edited. cyamus passes a generated override (in `$XDG_STATE_HOME/cyamus/compose/`) as the last `-f`.

The shared stack is configured the same whichever worktree starts it: it gets project-level variables only (no branch variables, no fingerprints, manifest `[vars]` as written).

### A typical setup

```toml
[[copy]]
source = "compose.yaml"
target = "compose.yaml"

[[hooks.on_setup]]
commands = ["cyamus compose up -d"]

[[hooks.on_teardown]]
commands = ["cyamus compose down"]
```

```yaml
# assets/compose.yaml
services:
  web:
    build: .
    ports: ["3000"]               # publish on a random host port; worktrees never collide
    environment:
      APP_URL: http://web.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}
  api:
    build: ./api
    ports: ["8080", "9229"]
    labels:
      cyamus.port: "8080"         # several published ports: pick the one to route
      cyamus.service: backend     # route as backend.<branch>.<project>.localhost
```

This produces `http://web.<branch>.<project>.localhost/` and `http://backend.<branch>.<project>.localhost/` in every worktree.

- **Publish ports without a host port** (`"3000"`, not `"3000:3000"`). Docker then picks a free port for each worktree, and the daemon finds it. A fixed host port collides as soon as two worktrees run the stack.
- **The repository already tracks a `compose.yaml`?** Copy your asset to `compose.override.yaml` instead. Compose merges it in automatically, so your labels and port changes apply without touching the team's file. Or skip the asset entirely, and let shared services replace the repo's own.
- **Use `cyamus compose` in your terminal too.** Plain `docker compose` doesn't know the stack's name, the shared services or the variables, so it would start a second stack. `cyamus daemon status` would then show its containers as duplicates.
- **Containers talk to each other by service name** (`http://api:8080`), as usual in compose. Inside a container, `*.localhost` is the container itself.
- **Only published ports are routed.** The daemon never uses container IPs, so it works the same with Docker Desktop, OrbStack, Colima and Linux. It finds Docker through `DOCKER_HOST`, then the current `docker context`, then `/var/run/docker.sock`.

| Label | Effect |
|---|---|
| `cyamus.enable` | `"false"` excludes the container. Everything else in a workspace is routed by default. |
| `cyamus.service` | Route name, instead of the compose service name |
| `cyamus.port` | Container port to route, when the container publishes more than one |
| `cyamus.project` + `cyamus.workspace` | Attach a container that compose didn't start in the worktree (e.g. `docker run`) to `<project>` and `<branch>`. Set both or neither. |

### Example: one Postgres, a database per worktree

One Postgres for the whole project, with its own database for each worktree. The repository's compose file can keep defining `db` for people who don't use cyamus; `cyamus compose` shadows it.

```yaml
# <config>/compose.yaml  (shared)
services:
  db:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD: postgres
    volumes: ["pgdata:/var/lib/postgresql/data"]
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "postgres"]
      interval: 2s
      retries: 15
    labels:
      cyamus.enable: "false"      # not HTTP; don't route it
  mailpit:
    image: axllent/mailpit
    ports: ["8025"]               # → http://mailpit.<project>.localhost/
volumes:
  pgdata: {}
```

```yaml
# compose.yaml  (the worktree's; for example the repository's own)
services:
  web:
    build: .
    ports: ["3000"]
    environment:
      DATABASE_URL: postgres://postgres:postgres@db:5432/app_${CYAMUS_BRANCH_SNAKE}
    depends_on: [db]
  db:                             # shadowed by the shared db
    image: postgres:16
```

```sh
#!/bin/sh
# bin/create_db: create this worktree's database once
db="app_${CYAMUS_BRANCH_SNAKE}"
cyamus compose-shared exec -T db psql -U postgres -tAc \
  "SELECT 1 FROM pg_database WHERE datname = '$db'" | grep -q 1 ||
  cyamus compose-shared exec -T db createdb -U postgres "$db"
```

```sh
#!/bin/sh
# bin/drop_db
cyamus compose-shared exec -T db dropdb -U postgres --if-exists --force "app_${CYAMUS_BRANCH_SNAKE}"
```

```toml
[[hooks.on_setup]]
commands = ["cyamus compose-shared up -d --wait", "create_db", "cyamus compose up -d"]

[[hooks.on_teardown]]
commands = ["cyamus compose down", "drop_db"]
```

The scripts expand `$CYAMUS_BRANCH_SNAKE` themselves, so each worktree's hooks create and drop that worktree's database. `cyamus compose-shared up -d --wait` makes sure Postgres is healthy before `create_db` runs.

### Recipe: per-worktree environment with direnv

Keep settings shared by every worktree in a linked `.env`, and let a setup hook write the per-worktree values (database name, service URLs) to `.env.local`. [direnv](https://direnv.net) loads both, with an `.envrc` that is the same in every worktree.

The project config directory then holds:

```
<config>/
├── cyamus.toml
├── assets/
│   ├── .env          ← your shared settings
│   ├── envrc         ← below
│   └── env.local     ← empty placeholder: touch assets/env.local
└── bin/
    └── write_env_local
```

Every asset source must exist, so create the empty placeholder with `touch assets/env.local`; setup fails on a missing source.

```sh
# assets/envrc
dotenv
dotenv_if_exists .env.local
```

```sh
#!/bin/sh
# bin/write_env_local
cat > .env.local <<EOF
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/app_${CYAMUS_BRANCH_SNAKE}
API_BASE_URL=http://api.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}
MAILPIT_URL=http://mailpit.${CYAMUS_PROJECT}.localhost${CYAMUS_URL_SUFFIX}
EOF
direnv allow .
```

```toml
[[link]]
source = ".env"
target = ".env"

[[link]]
source = "envrc"
target = ".envrc"

[[copy]]                     # the empty placeholder: keeps .env.local out of git status
source = "env.local"
target = ".env.local"

[[hooks.on_setup]]
commands = ["write_env_local"]
```

- **Order:** assets are applied before hooks, so copying the empty `env.local` makes cyamus exclude `.env.local` from `git status`, and the hook then writes the real content. Every setup resets it to empty and rewrites it, so it is always current.
- **Approval:** direnv asks for approval per directory, so the hook runs `direnv allow .` for each new worktree.
- **Your Mac, not containers:** these values are for tools that run on your machine. To reach the shared Postgres from there, publish it on a fixed host port in `<config>/compose.yaml` (`ports: ["5433:5432"]`; one per project, so it only has to be unique across projects). Containers keep using `db:5432`.
- **When values change:** they are written at setup. If you install or remove the [port-80 relay](#urls-without-a-port) later, re-run `cyamus workspace setup` to refresh `CYAMUS_URL_SUFFIX`.

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
