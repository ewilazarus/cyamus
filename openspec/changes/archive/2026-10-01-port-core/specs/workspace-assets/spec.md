# Spec Delta

## Purpose

Defines how files from a project's `assets/` directory are linked or copied into a workspace, and how those files are kept out of git status or are allowed to replace tracked files.

## ADDED Requirements

### Requirement: Asset path resolution
An asset's `source` SHALL be resolved relative to the project's `assets/` directory, and its `target` relative to the worktree root. A target that resolves outside the worktree root, or a source that resolves outside `assets/`, SHALL cause an error.

#### Scenario: Escaping target
- **WHEN** a `[[link]]` has `target = "../../etc/foo"`
- **THEN** setup fails with an error naming the entry and applies no assets

#### Scenario: Missing source
- **WHEN** a `[[copy]]` has a `source` that does not exist in `assets/`
- **THEN** setup fails with an error naming the missing source

### Requirement: Links
A `[[link]]` entry SHALL create a symbolic link at the target that points to the absolute path of the source, creating missing parent directories. A link that already points to the source SHALL be left as is.

#### Scenario: New link
- **WHEN** setup applies `[[link]] source = "env.local" target = "config/.env.local"` and `config/` does not exist
- **THEN** `config/` is created and `config/.env.local` is a symlink to `<assets>/env.local`

#### Scenario: Link already correct
- **WHEN** setup runs again and the target already links to the source
- **THEN** the link is left unchanged

#### Scenario: Conflicting existing target
- **WHEN** a non-override link's target already exists as a regular file or as a symlink pointing elsewhere
- **THEN** setup fails with an error naming the target and does not modify it

### Requirement: Copies
A `[[copy]]` entry SHALL copy the source to the target byte-for-byte, creating missing parent directories. A source directory SHALL be copied recursively. File permissions SHALL be preserved.

#### Scenario: File copy
- **WHEN** setup applies `[[copy]] source = "config.local.yaml" target = "config.local.yaml"`
- **THEN** the target is an independent regular file with the source's contents

#### Scenario: Directory copy
- **WHEN** the source is a directory `vscode/` and the target is `.vscode`
- **THEN** `.vscode/` contains copies of every file under `vscode/`, with the same relative paths

### Requirement: Copy overwrite control
A `[[copy]]` with `overwrite = true` (the default) SHALL replace an existing target on every setup. With `overwrite = false`, an existing target SHALL be left untouched, and the copy SHALL happen only when the target does not exist.

#### Scenario: Overwrite enabled
- **WHEN** the user edits a copied file and setup runs again with `overwrite = true`
- **THEN** the file is replaced with a fresh copy of the source

#### Scenario: Overwrite disabled
- **WHEN** the user edits a copied file and setup runs again with `overwrite = false`
- **THEN** the user's edits are preserved

### Requirement: Non-override targets are git-excluded
The targets of non-override assets SHALL be listed in a cyamus-managed block of the repository's shared `info/exclude` file, located in the git common dir. Paths are anchored at the worktree root. Each setup SHALL rewrite only that block and SHALL preserve all lines outside it.

#### Scenario: Targets excluded
- **WHEN** setup applies a non-override link to `.env.local`
- **THEN** `info/exclude` contains `/.env.local` inside the cyamus block, and `git status` in the worktree does not show `.env.local`

#### Scenario: User lines preserved
- **WHEN** `info/exclude` contains user-written lines before setup runs
- **THEN** those lines are unchanged afterwards

### Requirement: Non-override assets never replace tracked files
An asset without `override = true` whose target is a file tracked by git SHALL cause setup to fail before any asset is applied, leaving the tracked file unchanged.

#### Scenario: Copy onto a tracked file without override
- **WHEN** a `[[copy]]` targets the tracked file `tracked.txt` without `override = true`
- **THEN** setup fails with an error suggesting `override = true`, and `tracked.txt` is unchanged

### Requirement: Override mode replaces tracked files
An asset with `override = true` SHALL mark its target with git's skip-worktree flag, then replace the target with the link or copy, removing any existing file or directory. The target SHALL NOT be added to `info/exclude`. If the target is not a tracked file, setup SHALL fail.

#### Scenario: Override a tracked file
- **WHEN** `.vscode/settings.json` is tracked and a `[[link]]` targets it with `override = true`
- **THEN** the file is marked skip-worktree, replaced by the symlink, and `git status` shows no modification

#### Scenario: Override an untracked target
- **WHEN** an asset with `override = true` targets a path that git does not track
- **THEN** setup fails with an error suggesting removing `override`
