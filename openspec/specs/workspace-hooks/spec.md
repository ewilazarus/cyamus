# workspace-hooks Specification

## Purpose
Defines how the `on_setup` and `on_teardown` hook commands declared in the manifest are selected by branch conditions, resolved to bin scripts or inline shell commands, and executed.

## Requirements

### Requirement: Group selection by branch conditions
Hook groups SHALL be evaluated in declaration order. A group without `conditions` SHALL always run. A group with conditions SHALL run only when the branch matches `if_branch_matches` (if set) AND does not match `if_branch_not_matches` (if set). In patterns, `*` matches any sequence of characters including `/`, `?` matches one character, and `[...]` matches a character class.

#### Scenario: Unconditional group
- **WHEN** a group has no `conditions`
- **THEN** its commands run for every branch

#### Scenario: Match condition
- **WHEN** a group has `if_branch_matches = "gabriel/*"` and the branch is `gabriel/feat/x`
- **THEN** its commands run

#### Scenario: Both conditions are AND-ed
- **WHEN** a group has `if_branch_matches = "gabriel/*"` and `if_branch_not_matches = "gabriel/wip-*"`, and the branch is `gabriel/wip-1`
- **THEN** its commands do not run

### Requirement: Command resolution
Each command entry SHALL be run as the bin script `<config>/bin/<entry>` if a file with exactly that name exists there. Otherwise the entry SHALL be run as an inline command through the user's shell (`$SHELL -c`, falling back to `sh`).

#### Scenario: Bin script
- **WHEN** an entry is `install_deps` and `bin/install_deps` exists
- **THEN** `bin/install_deps` is executed

#### Scenario: Inline command
- **WHEN** an entry is `npm install` and no `bin/npm install` file exists
- **THEN** `npm install` is executed via the user's shell

#### Scenario: Non-executable bin script
- **WHEN** `bin/install_deps` exists but is not executable
- **THEN** the hook fails with an error saying the script is not executable

### Requirement: Execution context
Every hook command SHALL run with the worktree root as its working directory and the CYAMUS environment for the current event. The command's stdout and stderr SHALL be streamed to the user while it runs, attributed to the hook that produced them.

#### Scenario: Working directory
- **WHEN** a hook runs `pwd`
- **THEN** it prints the worktree root

#### Scenario: Output streaming
- **WHEN** a hook prints progressively over several seconds
- **THEN** the output appears as it is produced, not only after the hook exits

### Requirement: Sequential fail-fast execution
Commands SHALL run one at a time, in group order and then command order. If a command exits non-zero, no further commands SHALL run, and the invoking command SHALL fail and report the hook's name and exit code.

#### Scenario: Failure stops remaining hooks
- **WHEN** the first of three `on_setup` commands exits with code 1
- **THEN** the other two do not run and setup exits non-zero naming the first command

#### Scenario: No matching hooks
- **WHEN** no hook group matches the branch
- **THEN** setup or teardown completes successfully without running any command
