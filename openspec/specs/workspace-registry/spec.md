# workspace-registry Specification

## Purpose
Defines the on-disk registry of active workspaces that the CLI maintains during setup and teardown, and that the daemon reads to know which worktrees exist and what hostnames they own.

## Requirements

### Requirement: Registry location and record contents
Each registered workspace SHALL be stored as one TOML file at `$XDG_STATE_HOME/cyamus/workspaces/<project>/<branch-label>.toml` (default state home: `$HOME/.local/state`). The file SHALL contain the project name, the branch name, the branch label, and the canonical absolute path of the worktree root.

#### Scenario: Record written for a workspace
- **WHEN** setup completes registration for project `myproj`, branch `feature/My-Thing`, worktree `/w/feat`
- **THEN** `~/.local/state/cyamus/workspaces/myproj/feature-my-thing.toml` exists and records `myproj`, `feature/My-Thing`, `feature-my-thing` and `/w/feat`

#### Scenario: Symlinked worktree path
- **WHEN** the worktree is reached through a symlink (e.g. `/var/...` to `/private/var/...` on macOS)
- **THEN** the recorded path is the canonical one, with symlinks resolved

### Requirement: Branch label derivation
The branch label SHALL be the branch slug (`CYAMUS_BRANCH_SLUG`), truncated to 63 characters with trailing hyphens removed, so that it is a valid DNS label. A branch whose label would be empty SHALL NOT be registered, and setup SHALL print a warning saying the workspace has no routes.

#### Scenario: Long branch name
- **WHEN** the branch slug is 80 characters long
- **THEN** the label is its first 63 characters, without a trailing hyphen

#### Scenario: Branch with no usable characters
- **WHEN** the branch name contains no ASCII letters or digits
- **THEN** no record is written, setup warns, and setup otherwise continues

### Requirement: Atomic, serialized writes
Writes and removals SHALL hold an exclusive lock on `$XDG_STATE_HOME/cyamus/registry.lock`. Each record SHALL be written to a temporary file in the same directory and renamed into place, so readers never see a partial record.

#### Scenario: Concurrent setups
- **WHEN** Orca creates three worktrees of the same project at once
- **THEN** all three records are written completely and none is lost

### Requirement: Label conflicts within a project
If a record for the same project and branch label already exists for a *different* worktree path that still exists, setup SHALL keep the existing record, SHALL NOT register the new workspace, and SHALL print a warning naming both worktrees. If the existing record's path no longer exists, it SHALL be replaced.

#### Scenario: Two branches with the same slug
- **WHEN** worktree A on `Feature/X` is registered and setup runs for worktree B on `feature-x`
- **THEN** A's record is kept, B gets no record, and the warning names both paths

#### Scenario: Stale record for the same label
- **WHEN** a record points to a worktree that has been deleted, and a new worktree with the same label runs setup
- **THEN** the record is replaced with the new worktree's path

### Requirement: Re-registration is idempotent
Running setup again for a registered workspace SHALL rewrite its record with the same content and SHALL NOT produce a conflict warning.

#### Scenario: Repeated setup
- **WHEN** setup runs twice on the same worktree
- **THEN** one record exists and no warning is printed

### Requirement: Unregistration on teardown
Teardown SHALL remove the workspace's record if the record points to this worktree. A record that points to a different worktree SHALL be left alone. A missing record is not an error.

#### Scenario: Teardown removes its own record
- **WHEN** teardown runs for a registered worktree
- **THEN** its record file no longer exists

#### Scenario: Teardown does not remove another worktree's record
- **WHEN** teardown runs for worktree B, and the record for B's label belongs to worktree A
- **THEN** A's record is kept

### Requirement: Garbage collection
Every registry write or removal, and `cyamus daemon status`, SHALL first delete records whose worktree path no longer exists. Empty project directories under `workspaces/` SHALL be removed.

#### Scenario: Worktree deleted without teardown
- **WHEN** a worktree directory is deleted without teardown running, and setup later runs for another workspace
- **THEN** the deleted worktree's record is removed
