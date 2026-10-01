# Spec Delta

## Purpose

Identifies which cyamus project a git checkout belongs to, whether bare or regular, locates that project's configuration directory outside the repository, and lets the user open it for editing.

## ADDED Requirements

### Requirement: Project identity is read from git config
The system SHALL resolve a checkout's project name from the git config key `cyamus.project`, read through the checkout's git common dir, so every worktree of the same repository resolves to the same project regardless of whether the repository is bare or regular.

#### Scenario: Key set on a regular checkout
- **WHEN** `cyamus.project` is `myproj` in a regular clone's config and a command runs inside one of its linked worktrees
- **THEN** the resolved project is `myproj`

#### Scenario: Key set on a bare checkout
- **WHEN** `cyamus.project` is `myproj` in a bare repository's config and a command runs inside one of its worktrees
- **THEN** the resolved project is `myproj`

### Requirement: Project identity is guessed and persisted when missing
When `cyamus.project` is unset, any command that resolves the project (`workspace setup`, `workspace teardown`, `edit`) SHALL derive a name from the `origin` remote's repository name, or from the common dir's repository directory name if there is no `origin`. It SHALL normalize the name, write it to `cyamus.project` in the common dir's config, and report that it did so.

#### Scenario: Guess from origin
- **WHEN** `cyamus.project` is unset and `origin` is `git@github.com:acme/My_Proj.git`
- **THEN** setup writes `cyamus.project = my-proj`, reports the guessed name, and continues with project `my-proj`

#### Scenario: Guess without origin
- **WHEN** `cyamus.project` is unset, there is no `origin` remote, and the common dir is `/code/widgets.git`
- **THEN** setup writes `cyamus.project = widgets` and reports it

#### Scenario: Parallel first setups agree
- **WHEN** `cyamus.project` is unset and two setups run at the same time in different worktrees of the repository
- **THEN** both succeed and resolve the same project name

#### Scenario: Guess is persisted
- **WHEN** setup has guessed and written a project name, and setup later runs in another worktree of the same repository
- **THEN** the name is read from git config and no guess is reported

### Requirement: Project names are DNS-label safe
A project name SHALL consist only of lowercase ASCII letters, digits and hyphens, SHALL NOT start or end with a hyphen, and SHALL be at most 63 characters. Guessed names SHALL be normalized to meet this rule. An explicitly configured name that violates it SHALL cause an error.

#### Scenario: Invalid configured name
- **WHEN** `cyamus.project` is set to `My Proj!`
- **THEN** the command fails with an error naming the invalid value and the rule it violates

### Requirement: Project config directory location
A project's configuration directory SHALL be `$XDG_CONFIG_HOME/cyamus/projects/<project>/`. If `XDG_CONFIG_HOME` is unset or not an absolute path, `~/.config` SHALL be used, on every supported platform including macOS. The directory contains `cyamus.toml`, `assets/` and `bin/`.

#### Scenario: XDG_CONFIG_HOME unset on macOS
- **WHEN** `XDG_CONFIG_HOME` is unset and the project is `myproj`
- **THEN** the config directory is `~/.config/cyamus/projects/myproj/`

#### Scenario: XDG_CONFIG_HOME set
- **WHEN** `XDG_CONFIG_HOME=/tmp/cfg` and the project is `myproj`
- **THEN** the config directory is `/tmp/cfg/cyamus/projects/myproj/`

### Requirement: Edit opens the project config directory
`cyamus edit [path]` SHALL resolve the project for the given path (default: current directory) and launch `$EDITOR` with the project's config directory as its argument, waiting for the editor to exit. If the directory does not exist, it SHALL first create it with an empty `assets/`, an empty `bin/` and a minimal `cyamus.toml`.

#### Scenario: Existing config directory
- **WHEN** the user runs `cyamus edit` inside a worktree of project `myproj` and `EDITOR=nvim`
- **THEN** `nvim ~/.config/cyamus/projects/myproj` is launched

#### Scenario: Missing config directory is scaffolded
- **WHEN** the user runs `cyamus edit` and the project's config directory does not exist
- **THEN** the directory is created with `cyamus.toml`, `assets/` and `bin/`, and the editor is launched on it

#### Scenario: EDITOR unset
- **WHEN** `EDITOR` is unset or empty
- **THEN** the command fails with an error asking the user to set `EDITOR`, and creates nothing
