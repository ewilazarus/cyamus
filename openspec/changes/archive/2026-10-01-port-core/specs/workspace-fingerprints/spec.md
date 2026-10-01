# Spec Delta

## Purpose

Gives hooks a cheap way to tell whether their inputs changed: content hashes over sets of worktree files, plus a per-project cache directory that outlives worktrees, where hooks can store previous values.

## ADDED Requirements

### Requirement: Fingerprint computation
For each `[[fingerprint]]` entry, the system SHALL compute a SHA-256 digest over the entry's `files` sorted lexicographically. For each file it SHALL feed in the relative path's UTF-8 bytes followed by the file's contents. The lowercase hex digest SHALL be truncated to `length` characters.

#### Scenario: Same inputs give the same fingerprint
- **WHEN** two worktrees of a project have identical `package.json` and `package-lock.json`, and a fingerprint lists both
- **THEN** both worktrees get the same fingerprint value

#### Scenario: Changed input changes the fingerprint
- **WHEN** `package-lock.json` differs between two worktrees
- **THEN** their fingerprint values differ

#### Scenario: Declaration order does not matter
- **WHEN** one manifest lists `files = ["b", "a"]` and another lists `files = ["a", "b"]` over identical files
- **THEN** both produce the same fingerprint value

### Requirement: Missing files do not fail
A file listed in a fingerprint that is missing or unreadable SHALL contribute its relative path followed by the literal bytes `NULL` instead of its contents, and SHALL NOT cause an error.

#### Scenario: Missing file
- **WHEN** a fingerprint lists `Dockerfile` and the worktree has no `Dockerfile`
- **THEN** setup succeeds and the fingerprint is computed with the `NULL` marker for that file

### Requirement: Fingerprints are exposed as environment variables
Each fingerprint SHALL be exposed to hooks as `CYAMUS_FINGERPRINT_<NAME>`, where `<NAME>` is the fingerprint's `name` uppercased with every run of non-alphanumeric characters replaced by `_`.

#### Scenario: Fingerprint variable name
- **WHEN** a fingerprint is named `docker-deps`
- **THEN** hooks see `CYAMUS_FINGERPRINT_DOCKER_DEPS` set to its value

### Requirement: Fingerprints are computed after assets
During setup, fingerprints SHALL be computed after assets are applied and before `on_setup` hooks run, so files placed by assets are included. During teardown, they SHALL be computed before `on_teardown` hooks run.

#### Scenario: Fingerprint over a copied asset
- **WHEN** a fingerprint lists `config.local.yaml`, which is placed by a `[[copy]]` entry, and setup runs on a fresh worktree
- **THEN** the fingerprint reflects the copied file's contents, not the `NULL` marker

### Requirement: Project cache directory
Hooks SHALL receive `CYAMUS_CACHE_DIR` set to `$XDG_CACHE_HOME/cyamus/projects/<project>/`, falling back to `~/.cache` when `XDG_CACHE_HOME` is unset or not absolute. The directory SHALL exist before any hook runs. cyamus SHALL NOT delete or modify its contents.

#### Scenario: Cache directory created
- **WHEN** setup runs for project `myproj` with `XDG_CACHE_HOME` unset and `~/.cache/cyamus/projects/myproj/` missing
- **THEN** the directory is created and hooks see `CYAMUS_CACHE_DIR` pointing to it

#### Scenario: Cache outlives the worktree
- **WHEN** a hook writes a file to `CYAMUS_CACHE_DIR`, the worktree is torn down and deleted, and setup runs in a new worktree of the same project
- **THEN** the new worktree's hooks can read that file
