# Tasks

## 1. Scaffolding

- [x] 1.1 Create the Cargo workspace with `crates/cyamus-core` (lib) and `crates/cyamus-cli` (bin named `cyamus`), edition 2024, `rust-version` ≥ 1.89; verify `cargo build` succeeds and produces `target/debug/cyamus`
- [x] 1.2 Add the clap command tree (`workspace setup [path] [--var K=V]...`, `workspace teardown [path] [--var K=V]...`, `edit [path]`) with stub handlers; verify `cyamus --help` and `cyamus workspace --help` list exactly these commands
- [x] 1.3 Add `rustfmt`/`clippy` config and a CI workflow running fmt, clippy (`-D warnings`) and tests; verify the workflow passes locally via `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- [x] 1.4 Add the integration-test harness (`assert_cmd`, `tempfile`) with helpers that create a regular repo, a bare repo, linked worktrees via `git worktree add`, and an isolated `XDG_CONFIG_HOME`; verify a smoke test running `cyamus --help` passes

## 2. Git access and project config

- [x] 2.1 Implement the `Git` wrapper (toplevel, common dir, current branch / detached detection, config get/set `--local`, origin URL, tracked-file check, skip-worktree); verify unit/integration tests against regular and bare repos
- [x] 2.2 Implement XDG path resolution (`XDG_CONFIG_HOME` → `~/.config`, `XDG_CACHE_HOME` → `~/.cache`, ignoring non-absolute values) and the project config and cache dir layout; verify tests for set, unset and relative values of both variables
- [x] 2.3 Implement the slug normalizer and project-name validation (DNS label, ≤63, no leading/trailing hyphen); verify unit tests including `My_Proj` → `my-proj`, truncation, and rejection of `My Proj!`
- [x] 2.4 Implement project resolution: read `cyamus.project`, else guess from origin or common dir name, persist with `git config --local`, report on stderr; verify integration tests for the project-config spec scenarios (regular, bare, with/without origin, persisted guess)

## 3. Manifest

- [x] 3.1 Define the manifest types (version, vars, link, copy, fingerprint, hooks.on_setup/on_teardown, conditions) with `deny_unknown_fields` and defaults; verify parsing tests for every manifest spec scenario, including the `overwite` typo and `on_attach` rejection
- [x] 3.2 Implement the validation pass (unsupported version, variable-name and fingerprint-name collisions, empty fingerprint name, fingerprint length 1–64, asset and fingerprint path escape checks) that collects all errors; verify unit tests for each error
- [x] 3.3 Render parse errors with the manifest path and line/column; verify a test asserting the error text for a syntax error on a known line

## 4. Workspace resolution and environment

- [x] 4.1 Implement workspace resolution from a path (toplevel, branch, common dir, project, config dir), failing on non-git paths and detached HEAD; verify integration tests from a subdirectory, an explicit path, a non-repo path and a detached worktree
- [x] 4.2 Implement `--var KEY=VALUE` parsing and the CYAMUS environment builder (base vars, slug/snake, `CYAMUS_VAR_*` with runtime precedence, `CYAMUS_EVENT`); verify unit tests for the `feature/My-Thing` scenario and runtime override

## 5. Assets

- [x] 5.1 Implement links (absolute symlink, parent creation, already-correct no-op, conflict error); verify integration tests for each workspace-assets link scenario
- [x] 5.2 Implement copies (file and recursive directory, permission preservation, `overwrite` true/false); verify integration tests for file copy, directory copy and both overwrite modes
- [x] 5.3 Implement the locked, atomic `info/exclude` managed block with root-anchored targets; verify tests that `git status` hides excluded targets, user lines survive, and two concurrent setups leave a well-formed block
- [x] 5.4 Implement override mode (tracked check, skip-worktree, replace target, no exclude entry); verify integration tests that `git status` is clean after overriding a tracked file and that an untracked target errors

## 6. Fingerprints and cache directory

- [x] 6.1 Implement fingerprint computation (sorted files, path+content or `NULL`, SHA-256 via `sha2`, truncation) and `CYAMUS_FINGERPRINT_*` naming; verify unit tests with golden digests for present files, a missing file and reordered `files`
- [x] 6.2 Implement cache directory creation and `CYAMUS_CACHE_DIR`; verify an integration test where a hook writes a file in one worktree and a hook in a second worktree of the same project reads it after the first is removed

## 7. Hooks

- [x] 7.1 Implement group selection with `globset` (unconditional, matches, not-matches, AND); verify unit tests for each workspace-hooks condition scenario
- [x] 7.2 Implement command resolution (bin script vs `$SHELL -c`/`sh`, non-executable error) and execution (worktree cwd, inherited stdio, stderr header, fail-fast with hook name and exit code); verify integration tests for `pwd`, a failing middle hook, and a non-executable script

## 8. Lifecycle commands

- [x] 8.1 Wire `cyamus workspace setup`: resolve → load/validate manifest → assets → fingerprints + cache dir → `on_setup`, with the unconfigured-project notice and exit 0; verify integration tests for every setup scenario in the workspace-lifecycle spec, including repeated setup
- [x] 8.2 Wire `cyamus workspace teardown`: resolve → load/validate manifest → fingerprints + cache dir → `on_teardown`, never deleting anything; verify integration tests that the worktree, branch and assets remain after teardown and that hook failures exit non-zero
- [x] 8.3 Document setup/teardown, the CYAMUS environment, the manifest schema (including fingerprints) and a `CYAMUS_CACHE_DIR` + fingerprint skip-if-unchanged hook example in `README.md`, including the Orca setup/teardown script snippets; verify the README example manifest parses in a test

## 9. Edit command

- [x] 9.1 Implement `cyamus edit [path]`: resolve the project, scaffold the config dir (`cyamus.toml` with `version = 1`, `assets/`, `bin/`) if missing, launch `$EDITOR <dir>` and wait, erroring when `EDITOR` is unset; verify integration tests using a stub `EDITOR` script that records its argument
- [x] 9.2 Document `cyamus edit` and the config directory layout in `README.md`; verify the documented paths match the integration test expectations

## 10. End-to-end check

- [x] 10.1 Add an end-to-end test that imitates Orca against a bare and a regular repo: `git worktree add`, `cyamus workspace setup` (guessing the project), edit the manifest, run setup again, `cyamus workspace teardown`, then `git worktree remove`; verify it passes on macOS
- [x] 10.2 Manually run the Orca flow by replacing the git-workspace calls in one real project's Orca setup/teardown scripts with cyamus; verify a worktree created and closed in Orca's UI gets its assets and runs its hooks
