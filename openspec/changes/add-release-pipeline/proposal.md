# Proposal

## Why

Today the only way to install cyamus is to clone the repository and run `cargo install --path crates/cyamus-cli`, which requires a Rust toolchain. Users configuring Orca lifecycle scripts should be able to get a prebuilt `cyamus` binary with one `curl` command.

## What Changes

- New GitHub Actions workflow `.github/workflows/release.yml`, triggered by pushing a `v*` tag:
  - Fails early if the tag does not equal `v` + the `[workspace.package].version` in `Cargo.toml`.
  - Runs lint and tests before building.
  - Builds `cyamus` in release mode on native runners for three targets: `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. Intel macOS and Windows are out of scope.
  - Packages each binary as `cyamus-<target>.tar.gz` with a matching `.sha256` file and publishes them to a GitHub Release for the tag.
  - Uploads `install.sh` to the same release.
- New POSIX `install.sh` at the repository root, served from `https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh`. It:
  - detects the OS and architecture;
  - picks the latest release, or the one named by `$CYAMUS_VERSION`;
  - downloads the tarball and verifies its SHA-256;
  - installs to `~/.local/bin`, or to `$CYAMUS_INSTALL_DIR`;
  - prints a hint when that directory is not on `PATH`, without editing shell rc files.
- README changes:
  - "Installation" leads with the curl one-liner and documents the overrides.
  - `cargo install` moves under "From source".
  - "Platform support" lists the prebuilt targets.

## Capabilities

### New Capabilities
- `release-builds`: How tagged versions turn into published, checksummed prebuilt binaries: tag/version consistency, the target set, artifact naming and the release contents.
- `install-script`: The behavior of the curl-pipeable installer: platform detection, version selection, checksum verification, install location and PATH guidance.

### Modified Capabilities
<!-- none -->

## Impact

- New files: `.github/workflows/release.yml`, `install.sh`.
- Modified: `README.md`. The existing `ci.yml` is unchanged.
- No Rust code or dependency changes. The toolchain used in release builds is pinned to 1.97.1 to match `devbox.json`.
- Cutting a release means bumping the workspace version, committing, and pushing a matching `vX.Y.Z` tag.
