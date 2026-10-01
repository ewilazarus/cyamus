# Design

## Context

- git-workspace's `compose` ran `docker compose -p <name> -f <config>/compose.yml <args>` for shared services. Getting worktree stacks to use those services took a hand-made network, a `networks:` block in both compose files, and an "omitted" profile on duplicated services.
- `add-daemon-proxy` already exports `COMPOSE_PROJECT_NAME=<project>-<label>`, `CYAMUS_DOMAIN`, `CYAMUS_PROXY_PORT` and `CYAMUS_URL_SUFFIX` to hooks. It routes containers by compose `working_dir`, and reserved `<svc>.<project>.localhost` for shared services.
- Spikes run during exploration (OrbStack, Compose 5.1.2):
  - **Network attach.** A running worktree container attached to the shared stack's network resolves the shared `db` by name (`getent hosts db`, and a TCP round trip worked). It appears there only under its container name and ID, never its service name, so worktrees can't collide on that network.
  - **Shadowing by the worktree's own service.** If the worktree also runs its own `db`, its containers keep resolving `db` to their own instance even after the attach. Duplicates must therefore be removed, not just routed around.
  - **Removing networks.** `docker compose down` of the worktree stack removed its network cleanly with containers attached elsewhere. `down` of the **shared** stack failed with "Resource is still in use" while a worktree container was attached to the shared stack's own default network.
  - **Disabling a dependency.** An override giving `db` a profile, while `web` still `depends_on: db`, makes compose reject the project: `service "web" depends on undefined service "db"`. Adding `web: depends_on: !override {cache: …}` to the same override works: `web` and `cache` start, and `db` doesn't.
  - **Reading the model.** `docker compose config --format json` returns the resolved services and their `depends_on`.
  - **Model shape (spike 1.2).**
    - An implicit default network appears as `"networks": {"default": null}`; explicit networks keep their settings (aliases and so on).
    - `network_mode` services have no `networks` key, and `network_mode: service:x` adds an implicit `depends_on: x`.
    - `depends_on` is normalized to `{condition, required, restart}`.
    - Profiled services only appear with `--profile '*'`, which cyamus therefore uses when reading the model.
    - Interpolation uses the calling environment: missing variables become blank, with a warning.
  - **End to end (spike 1.3, two stacks).** The worktree `db` was not started, and `web` reached the shared `db` and `mailpit` by name and waited for `cache` to become healthy. `worker` kept `backend` and its `jobs` alias without being added to `default`, and `network_mode: host` was untouched. Both stacks' `down` worked in either order.

## Goals / Non-Goals

**Goals:**
- Zero edits to the repository's compose files for shared services to replace per-worktree duplicates.
- `cyamus compose …` behaves like `docker compose …` (files, arguments, exit code) plus the cyamus behaviour. Running it from a hook or a terminal gives identical results.
- `cyamus-core` stays synchronous, with no Docker crates; Docker is driven through its CLI, as git-workspace did.

**Non-Goals:**
- Per-branch opt-out of shadowing (e.g. a private database for one branch). It can be added later through manifest conditions.
- Starting the shared stack from `workspace setup` itself.
- Managing databases inside shared services. The README shows the hook pattern instead.
- Supporting Docker Compose v1 (`docker-compose`) or Compose versions before 2.24.4.

## Decisions

### Generated override, not edited files
Each invocation builds a small override and passes it as the **last** `-f`, so it wins merges:
```yaml
# $XDG_STATE_HOME/cyamus/compose/myproj/feat-x.yaml  (worktree stack)
services:
  "db":                       # shadowed: also defined in the shared stack
    profiles: ["cyamus-shadowed"]
  "web":
    depends_on: !override {"cache": {"condition": "service_healthy", "required": true, "restart": false}}
    networks: !override {"default": null, "cyamus-net": null}   # own networks + the worktree's cyamus network
networks:
  "cyamus-net": {"name": "cyamus-myproj-feat-x", "external": true}
```
`!override` replaces a key instead of merging it, so the generated values must be complete:
- **`depends_on`** is copied from the resolved model with the shadowed entries removed, keeping each remaining entry's `condition`, `required` and `restart`.
- **`networks`** is the resolved map of the service's own networks, with their settings, plus `cyamus-net`. Spike 1.2 showed an implicit default appears as `{"default": null}`. A plain merge can't be used here: adding one key to a service that relies on the implicit default would detach it from `default`.

Services with `network_mode` are skipped, since they can't join networks. **The override is YAML whose values are JSON taken straight from the resolved model.** JSON is valid YAML flow syntax, so only the `!override` tags and quoted keys are hand-written, and no user data is ever interpreted. Spike 1.3 confirmed Compose 5.1.2 accepts this form.

Override files live in `$XDG_STATE_HOME/cyamus/compose/<project>/<label>.yaml`, one per worktree. The shared stack needs none. They are rewritten on every invocation: cheap, and never stale.

*Alternatives:*
- Materializing one merged file (`docker compose config` over all files, then running against the output): rejected. Stacking `-f` keeps compose's own handling of relative paths, `env_file`, the project directory and `watch`; reports errors against the user's files; and avoids an extra `config` call and a copy that can go stale. `docker compose config` is still used to *read* the merged model and to validate overrides in tests. (Compose 5.1.2 has no separate `merge` subcommand.)
- Exporting `COMPOSE_FILE` to hooks only: breaks manual terminal use.
- The daemon attaching networks after start: an app can boot before `db` resolves (the connect took 67 ms), and it can't remove duplicates.
- Profiles alone: rejected by compose when `depends_on` points at the disabled service.

### Reading the model
cyamus runs `docker compose -p <name> <files> config --format json` with the same environment and files as the real invocation, and parses `services.*.depends_on`, `services.*.networks` and `services.*.network_mode`. The worktree's service names and the shared stack's service names come from the same call on each stack, and shadowed = their intersection. Services that only an inactive profile would enable must still be considered. The spike (1.2) determines the right flags (e.g. `--profile '*'`) so the profile check doesn't hide them.

### One network per worktree, owned by cyamus
*First design, rejected by spike 1.3:* one project-wide network `cyamus-<project>` that every service of both stacks joined. Compose gives each service its service-name alias on **every** network it joins. With two worktrees on the shared network, `web` in worktree A resolved `api` to A's own `api` 13 times and **B's `api` 7 times** out of 20 (Docker's DNS answered from the shared network, never from A's default network). A could also resolve names only B defines.

*Chosen:* a network per worktree, `cyamus-<project>-<label>`, labelled `dev.cyamus.project` / `dev.cyamus.workspace`, holding only that worktree's services and the shared ones.
- **Creation.** `cyamus compose` creates it if missing. The override declares it external, and every worktree service joins it (as `cyamus-net`) alongside its own networks.
- **Attaching the shared stack.** Before `docker compose` runs, cyamus connects each running shared container to it with `docker network connect --alias <service>`, skipping containers already connected. When an app in the worktree first connects to `db`, the alias already exists, so there is no start-up race.
- **Teardown.** After a successful `cyamus compose down`, cyamus disconnects the shared containers and removes the network. Failures are warnings. A plain `docker compose down` leaves the external network behind, which is harmless.
- **Reconcile.** Shared containers recreated by compose lose runtime attachments. So after every `cyamus compose-shared` invocation, and after the auto-start in `cyamus compose up`, cyamus connects every running shared container to every network labelled with the project. It costs one `docker network ls` plus one inspect per network.
- **Shared stack.** It keeps its own compose network for internal traffic and needs no override for networking. Its override is empty unless later features need one.
- **Removal.** Worktree `down` doesn't try to remove an external network, so attached shared containers can't block it. Shared `down` removes only the shared stack's own network, which no worktree joined. Both orders were verified in spike 1.3.

### Compose file discovery
Passing any `-f` disables compose's own discovery, including the automatic override file. So when the user passes no `-f`/`--file` and `COMPOSE_FILE` is unset, cyamus reproduces it. Spike 1.1 recorded Compose 5.1.2's behaviour:

| Case | Compose uses |
|---|---|
| One default name present | that file |
| Several base names | the first of `compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml` (with a warning). Note `.yml` before `.yaml` for the `docker-compose` names. |
| Override files | **one**, the first of `compose.override.yml`, `compose.override.yaml`, `docker-compose.override.yml`, `docker-compose.override.yaml`, regardless of which base name won. Here `.yml` comes first for both prefixes. |
| Override only in a subdirectory of the base file's directory | not used: overrides are looked up next to the chosen base file only |
| Base file in a parent directory | found: the search goes upward... |
| ...past a nested git repository root | ...and does **not** stop at git or worktree roots; it continues to `/` |
| `COMPOSE_FILE=a.yaml:b.yaml` | `a.yaml` then `b.yaml`, and no automatic override |
| `-f compose.yaml` | that file only; the automatic override is skipped |

cyamus mirrors this table in one function, with a test per row:
- **Upward search.** The search runs to `/`, not to the worktree root as first planned, because the goal is identical behaviour to `docker compose`.
- **`COMPOSE_FILE`** is split by `COMPOSE_PATH_SEPARATOR` (default `:`).
- **User `-f`/`--file` arguments**, in any of compose's forms (`-f x`, `--file x`, `--file=x`, `-fx`), are kept in place, and cyamus's `-f` is appended after them.

Arguments are split at the first non-option token, the compose subcommand. Global options before it (such as `-f` or `--profile`) are forwarded as global options, and everything after it is left untouched. That is how `cyamus compose -f x.yaml up -d web` keeps working.

### Starting the shared stack on `up` and `run`
The check is `config --services` for the shared stack compared with `ps --status running --services`. If any service is missing, cyamus runs `up -d` for the shared stack, with a one-line notice on stderr ("cyamus: starting the shared stack for myproj"), and stops on failure. It runs only for the `up` and `run` subcommands, because those start dependencies. Other subcommands never touch the shared stack, so one worktree's `down` can't stop shared services.

### Environment
The variables come from the existing `env::build`; the compose commands now load the manifest and compute fingerprints, as setup does. The two stacks follow different rules:
- **Worktree stack: fill in what's missing.** cyamus adds each variable only if the environment doesn't already have it. Inside a hook, the inherited values, including runtime `--var` overrides, therefore win. A terminal gets the manifest values, so hooks and terminals agree except for runtime overrides, which only exist during a setup or teardown call.
- **Shared stack: set explicitly, strip the worktree variables.** The first `cyamus compose up` in *any* worktree may start the shared stack, so its environment must not depend on which one. cyamus sets the project-level values and removes `CYAMUS_BRANCH*`, `CYAMUS_WORKSPACE`, `CYAMUS_EVENT`, `CYAMUS_FINGERPRINT_*`, inherited `CYAMUS_VAR_*` and `COMPOSE_PROJECT_NAME`. `CYAMUS_DOMAIN` becomes `<project>.localhost`, so `http://<svc>.${CYAMUS_DOMAIN}${CYAMUS_URL_SUFFIX}` is the right URL in both stacks.
- **Project names** are passed as `-p`. The `COMPOSE_PROJECT_NAME` from the environment is honoured only for the worktree stack, as in hooks.
- **URL suffix:** `CYAMUS_URL_SUFFIX` uses the same port-80 detection as setup.

### Daemon: shared-stack membership
`resolve` gains a list of `(project, canonical config dir)` pairs, built from `$XDG_CONFIG_HOME/cyamus/projects/*` and re-listed when the registry changes (cheap). A container without explicit workspace labels, whose canonical `working_dir` equals a project's config directory, becomes a shared route `<svc>.<project>.localhost`. `Route.workspace` becomes optional, which shows up in the status JSON and the CLI rendering ("myproj/shared"). Every other routing rule (opt-out, ports, duplicates, invalid labels) applies unchanged.

## Risks / Trade-offs

- [Compose's file discovery changes in a future version.] → It is mirrored in one function with a test per rule, and spike 1.1 pins today's behaviour. `-f` and `COMPOSE_FILE` remain escape hatches.
- [A name collision the user didn't intend: the worktree's `db` and the shared `db` are different things.] → Shadowing is by name, by design. The README states it, and `cyamus compose up` prints the shadowed services ("using shared: db, redis"), so it's visible.
- [Shared services keep running after the last worktree is torn down.] → Intended; `cyamus compose-shared down` stops them. The README says so.
- [`!override` needs Compose 2.24.4 or later.] → A version check with a clear error. Compose 2.24.4 shipped in January 2024.
- [`docker compose config` adds latency to every invocation (two calls, roughly 100–200 ms).] → Acceptable for a command that starts containers. It can be cached by file mtimes later if it ever bothers anyone.
- [Hooks call `cyamus` itself.] → Hooks already inherit `PATH`. The README's examples use `cyamus compose`, and `cyamus` is on `PATH` wherever Orca invokes it.
