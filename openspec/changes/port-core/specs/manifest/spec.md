# Spec Delta

## Purpose

Defines the `cyamus.toml` manifest that describes how every workspace of a project is set up and torn down, and how invalid manifests are reported.

## ADDED Requirements

### Requirement: Manifest location and format
Each project's manifest SHALL be the TOML file `cyamus.toml` at the root of the project config directory.

#### Scenario: Manifest is read from the project config directory
- **WHEN** a command needs the manifest for project `myproj`
- **THEN** it reads `$XDG_CONFIG_HOME/cyamus/projects/myproj/cyamus.toml`

### Requirement: Manifest version
The manifest SHALL accept an optional integer `version` key, defaulting to `1`. A version higher than the highest version this build supports SHALL cause an error.

#### Scenario: Version omitted
- **WHEN** the manifest has no `version` key
- **THEN** it is treated as version 1

#### Scenario: Unsupported version
- **WHEN** the manifest declares `version = 2` and the build supports only version 1
- **THEN** the command fails with an error stating the supported version

### Requirement: Variables
The manifest SHALL accept a `[vars]` table of string keys to string values. Two keys that normalize to the same environment variable name (uppercased, every non-alphanumeric run replaced with `_`) SHALL cause an error.

#### Scenario: Variables declared
- **WHEN** the manifest contains `[vars]` with `node-version = "22"`
- **THEN** the variable is available to hooks as `CYAMUS_VAR_NODE_VERSION=22`

#### Scenario: Colliding variable names
- **WHEN** `[vars]` contains both `node-version` and `node_version`
- **THEN** the command fails with an error naming both keys

### Requirement: Asset entries
The manifest SHALL accept `[[link]]` and `[[copy]]` arrays. Each entry SHALL have a `source` and a `target` (relative paths) and an optional boolean `override` (default `false`). `[[copy]]` entries SHALL also accept an optional boolean `overwrite` (default `true`).

#### Scenario: Link and copy declared
- **WHEN** the manifest declares a `[[link]]` with `source = "env.local"`, `target = ".env.local"` and a `[[copy]]` with `overwrite = false`
- **THEN** both entries are parsed with the given values and defaults for omitted keys

#### Scenario: Missing required key
- **WHEN** a `[[copy]]` entry has no `target`
- **THEN** the command fails with an error identifying the entry and the missing key

### Requirement: Hook groups
The manifest SHALL accept `[[hooks.on_setup]]` and `[[hooks.on_teardown]]` arrays of hook groups. Each group SHALL have a `commands` list of strings and an optional `conditions` table with `if_branch_matches` and/or `if_branch_not_matches` glob strings.

#### Scenario: Hook groups declared
- **WHEN** the manifest declares two `[[hooks.on_setup]]` groups, one with `conditions = { if_branch_matches = "gabriel/*" }`
- **THEN** both groups are parsed in declaration order, and only the second has conditions

### Requirement: Fingerprint entries
The manifest SHALL accept a `[[fingerprint]]` array. Each entry SHALL have a non-empty `name`, a `files` list of paths relative to the worktree root, and an optional integer `length` from 1 to 64 (default `12`). A `files` entry that escapes the worktree root, or two names that normalize to the same variable name, SHALL cause an error.

#### Scenario: Fingerprint declared with defaults
- **WHEN** a `[[fingerprint]]` has `name = "docker-deps"` and `files = ["Dockerfile", "package.json"]`
- **THEN** it is parsed with `length = 12`

#### Scenario: Invalid fingerprint
- **WHEN** a `[[fingerprint]]` has an empty `name`, `length = 0`, or a file `../secret`
- **THEN** the command fails with an error naming the entry and the problem

#### Scenario: Colliding fingerprint names
- **WHEN** two fingerprints are named `docker-deps` and `docker_deps`
- **THEN** the command fails with an error naming both

### Requirement: Unknown keys are rejected
The manifest SHALL be rejected when it contains keys that are not part of the schema at any level, including removed git-workspace keys such as `base_branch`, `[prune]`, `on_attach` and `on_detach`.

#### Scenario: Typo in a key
- **WHEN** a `[[copy]]` entry contains `overwite = false`
- **THEN** the command fails with an error naming the unknown key and its location

#### Scenario: Removed git-workspace key
- **WHEN** the manifest contains `[[hooks.on_attach]]`
- **THEN** the command fails with an error naming `on_attach` as unknown

### Requirement: Parse errors are reported with location
A manifest that is not valid TOML or does not match the schema SHALL cause the command to fail with a non-zero exit code and an error message that includes the manifest path and, where available, the line and column.

#### Scenario: Invalid TOML
- **WHEN** `cyamus.toml` contains a syntax error on line 4
- **THEN** the command exits non-zero and the error mentions the manifest path and line 4
