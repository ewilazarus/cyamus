# Spec Delta

## Purpose

Defines how a tagged version of cyamus becomes a GitHub Release containing checksummed prebuilt binaries for every supported platform, plus the installer script.

## ADDED Requirements

### Requirement: Release trigger
A release SHALL be produced when a tag matching `v*` is pushed to the repository. Pushing branches or other tags SHALL NOT produce a release.

#### Scenario: Version tag pushed
- **WHEN** tag `v0.2.0` is pushed
- **THEN** the release pipeline runs for that tag

#### Scenario: Branch push
- **WHEN** a commit is pushed to `main` without a tag
- **THEN** no release is created

### Requirement: Tag matches crate version
The release SHALL fail without publishing anything if the tag is not exactly `v` followed by the `[workspace.package].version` in `Cargo.toml`.

#### Scenario: Matching tag
- **WHEN** `Cargo.toml` declares version `0.2.0` and tag `v0.2.0` is pushed
- **THEN** the release proceeds

#### Scenario: Mismatched tag
- **WHEN** `Cargo.toml` declares version `0.1.0` and tag `v0.2.0` is pushed
- **THEN** the release fails with an error naming both versions, and no GitHub Release is created

### Requirement: Quality gate
The release SHALL run the project's formatting check, lint and test suite on the tagged commit. If any of them fails, nothing SHALL be published.

#### Scenario: Tests fail on tagged commit
- **WHEN** a test fails on the tagged commit
- **THEN** no binaries are built or published

### Requirement: Supported targets
Every release SHALL include a prebuilt binary for each of `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. Linux binaries SHALL be statically linked. If any target fails to build, nothing SHALL be published.

#### Scenario: All targets present
- **WHEN** a release is published
- **THEN** it contains an archive for each of the three targets

#### Scenario: Linux binary runs without a specific libc
- **WHEN** the Linux binary is run on a distribution with any libc, or none
- **THEN** it starts without dynamic-linker errors

### Requirement: Artifact naming and contents
Each target's archive SHALL be named `cyamus-<target>.tar.gz` and SHALL contain a single executable file named `cyamus` at the archive root.

#### Scenario: Archive layout
- **WHEN** `cyamus-aarch64-apple-darwin.tar.gz` is extracted
- **THEN** it yields one executable file `cyamus`, and `./cyamus --version` prints the release version

### Requirement: Checksums
Each archive SHALL be published with a companion `cyamus-<target>.tar.gz.sha256` file containing the archive's SHA-256 hex digest followed by its file name, in the format `sha256sum` output uses.

#### Scenario: Checksum verifies
- **WHEN** both the archive and its `.sha256` file are downloaded and `sha256sum -c` is run on the checksum file
- **THEN** verification succeeds

### Requirement: Installer published with release
Every release SHALL include the repository's `install.sh` as an asset, so that `https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh` always serves the installer from the latest release.

#### Scenario: Latest installer URL
- **WHEN** a user fetches `…/releases/latest/download/install.sh` after `v0.2.0` is published
- **THEN** they receive the `install.sh` from the `v0.2.0` tag
