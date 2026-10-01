---
name: cyamus
description: Use when working in a git worktree of a cyamus project (`git config cyamus.project` prints a name), typically one created by Orca. Use it to start or stop the worktree's Docker services, find the URLs they are served at, reach shared services such as the project database, re-apply the worktree's setup, or find out why a route or hook isn't working.
---

# cyamus

[cyamus](https://github.com/ewilazarus/cyamus) sets up each worktree's environment. `git config cyamus.project` names the project. Its configuration lives **outside the repository**, in `~/.config/cyamus/projects/<project>/` (`cyamus edit` opens it):

- `cyamus.toml`: the files linked or copied into every worktree, plus the setup and teardown hooks.
- `assets/`: the sources of those files.
- `bin/`: hook scripts.
- `compose.yaml` (optional): services shared by every worktree of the project, such as a database.

## Common tasks

| Goal | Command |
|---|---|
| Start this worktree's services | `cyamus compose up -d` |
| Logs, status, a shell | `cyamus compose logs -f web`, `cyamus compose ps`, `cyamus compose exec web sh` |
| Stop this worktree's services | `cyamus compose down` |
| Inspect shared services (database, cache, mail) | `cyamus compose-shared ps`, `cyamus compose-shared exec db psql -U postgres` |
| Find the URL of a service | `cyamus daemon status` lists every route; or open `http://cyamus.localhost/` |
| Re-apply links, copies and setup hooks | `cyamus workspace setup` (safe to repeat) |

`cyamus compose …` and `cyamus compose-shared …` take the same arguments as `docker compose …`.

## URLs

Each service of a worktree is served at:

```
http://<service>.<branch>.<project>.localhost/
```

For example, `http://web.feat-login.shop.localhost/`. `<branch>` is the branch name lowercased, with every run of other characters replaced by `-`. Shared services are at `http://<service>.<project>.localhost/`. If `cyamus daemon status` reports `port 80: not redirected`, add `:1355` after the hostname.

These URLs work from the host: browsers, `curl`, tests running on the machine. **Inside a container**, reach other services by their compose service name instead (`http://api:8080`, `db:5432`).

## Rules

- **Always use `cyamus compose`, never `docker compose`, for this worktree's services.** Plain `docker compose` starts a second, unnamed stack: it doesn't use the shared services, and the second stack's routes collide with the first.
- **Don't stop or recreate the shared stack** (`cyamus compose-shared down`, `… up --force-recreate`) unless asked: every worktree of the project uses it.
- **Services named in the shared `compose.yaml` aren't started per worktree.** `cyamus compose` prints them (`using shared: db`). The worktree reaches the shared ones by the same name, so don't "fix" a missing `db` container.
- **Don't create or remove worktrees, and don't run `cyamus workspace teardown`.** Orca owns worktrees. Teardown hooks may delete per-worktree data, such as the worktree's database.
- **Files cyamus links or copies into the worktree are excluded from git**, through a cyamus block in the repository's `info/exclude`. Don't commit them, and don't edit linked files expecting a local change: a linked file is a symlink shared by every worktree. To change one for good, edit its source in the config directory's `assets/`.
- **Don't run `cyamus daemon install` or `uninstall`** unless asked. They change system configuration with `sudo`.

## When something doesn't work

| Symptom | Meaning |
|---|---|
| `404` from a `*.localhost` URL, listing routes | No such route: check the service name and branch in `cyamus daemon status` |
| `502` from a route | The container is routed but not answering yet (still starting, or crashed): check `cyamus compose logs <service>` |
| A service missing from `cyamus daemon status` | See its line under "not routed": no published port, several ports without a `cyamus.port` label, or opted out |
| `cyamus daemon status` says not running | Run `cyamus workspace setup`; it starts the daemon |
| A link or copy is missing in the worktree | Run `cyamus workspace setup`; it re-applies everything |
