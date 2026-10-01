# Spec Delta

## Purpose

Defines the installer that users pipe from `curl` into `sh` to download, verify and install a prebuilt `cyamus` binary without a Rust toolchain.

## ADDED Requirements

### Requirement: POSIX shell compatibility
The installer SHALL run under any POSIX `sh`, with no bash-specific features. Besides standard POSIX utilities it SHALL require only `curl`, `tar`, and either `sha256sum` or `shasum`.

#### Scenario: Piped into sh
- **WHEN** a user runs `curl -fsSL https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh | sh`
- **THEN** the installer runs to completion under `/bin/sh` on macOS and Linux

#### Scenario: Missing required tool
- **WHEN** neither `sha256sum` nor `shasum` is available
- **THEN** the installer exits non-zero, names the missing tool, and installs nothing

### Requirement: Platform detection
The installer SHALL map the host OS and CPU architecture to a release target. Darwin/arm64 maps to `aarch64-apple-darwin`. Linux/x86_64 maps to `x86_64-unknown-linux-musl`. Linux/aarch64 or arm64 maps to `aarch64-unknown-linux-musl`. Any other combination SHALL be rejected.

#### Scenario: Apple Silicon Mac
- **WHEN** the installer runs on macOS on arm64
- **THEN** it selects `cyamus-aarch64-apple-darwin.tar.gz`

#### Scenario: Unsupported platform
- **WHEN** the installer runs on macOS on x86_64
- **THEN** it exits non-zero with a message naming the unsupported OS/architecture and pointing to installing from source

### Requirement: Version selection
The installer SHALL install the latest release by default. If `CYAMUS_VERSION` is set, it SHALL install that release instead, accepting the value with or without a leading `v`.

#### Scenario: Default version
- **WHEN** `CYAMUS_VERSION` is unset
- **THEN** the binary is downloaded from the latest release

#### Scenario: Pinned version
- **WHEN** `CYAMUS_VERSION=0.2.0` (or `v0.2.0`) is set
- **THEN** the binary is downloaded from release `v0.2.0`

#### Scenario: Nonexistent version
- **WHEN** `CYAMUS_VERSION` names a release that does not exist
- **THEN** the installer exits non-zero with a download error and installs nothing

### Requirement: Checksum verification
The installer SHALL download the archive's `.sha256` file and verify the archive against it before extracting. On mismatch it SHALL exit non-zero and install nothing.

#### Scenario: Corrupted download
- **WHEN** the downloaded archive does not match its published SHA-256
- **THEN** the installer exits non-zero, reports a checksum mismatch, and leaves any existing `cyamus` untouched

### Requirement: Install location
The installer SHALL install the binary as `cyamus` in `$CYAMUS_INSTALL_DIR` if set, otherwise in `$HOME/.local/bin`. It SHALL create the directory if needed, never invoke `sudo`, and replace an existing `cyamus` there.

#### Scenario: Default location
- **WHEN** `CYAMUS_INSTALL_DIR` is unset
- **THEN** the binary is installed at `$HOME/.local/bin/cyamus` and is executable

#### Scenario: Custom location
- **WHEN** `CYAMUS_INSTALL_DIR=/opt/tools/bin` is set
- **THEN** the binary is installed at `/opt/tools/bin/cyamus`

#### Scenario: Upgrade
- **WHEN** an older `cyamus` already exists in the install directory
- **THEN** it is replaced with the downloaded version

### Requirement: PATH guidance
After installing, the installer SHALL report the installed path and version. If the install directory is not on `PATH`, it SHALL print the line the user would add to their shell profile. It SHALL NOT modify any shell profile or rc file itself.

#### Scenario: Install directory not on PATH
- **WHEN** `$HOME/.local/bin` is not in `PATH`
- **THEN** the installer prints a hint such as `export PATH="$HOME/.local/bin:$PATH"` and leaves `~/.zshrc`, `~/.bashrc` and other profiles unchanged

### Requirement: No partial state on failure
The installer SHALL stop at the first error, work in a temporary directory that it removes on exit, and move the binary into the install directory only after every check has passed.

#### Scenario: Network failure mid-download
- **WHEN** the download fails partway
- **THEN** the installer exits non-zero, the temporary directory is removed, and the install directory is unchanged
