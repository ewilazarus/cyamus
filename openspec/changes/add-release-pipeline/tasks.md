# Tasks

## 1. Installer script

- [x] 1.1 Write `install.sh` at the repo root (POSIX sh, `set -eu`, temp dir with an EXIT trap, required-tool checks for `curl`, `tar` and `sha256sum`/`shasum`). Verify with `shellcheck -s sh install.sh` (no warnings) and `checkbashisms` or `dash -n install.sh`.
- [x] 1.2 Implement platform detection (Darwin arm64, including the Rosetta check; Linux x86_64, aarch64 and arm64) and an unsupported-platform error that points to installing from source. Verify by running the detection with stubbed `uname` output for each mapping and for Darwin/x86_64.
- [x] 1.3 Implement version selection (latest via `releases/latest/download`, or `CYAMUS_VERSION` with an optional `v`), download, checksum verification, and atomic install into `${CYAMUS_INSTALL_DIR:-$HOME/.local/bin}` with a PATH hint. Verify against a local fixture: serve a hand-built tarball and `.sha256` with `python3 -m http.server` through a base-URL override, and check a successful install, a tampered tarball (non-zero exit, existing binary untouched), and a custom `CYAMUS_INSTALL_DIR`.

## 2. Release workflow

- [x] 2.1 Add `.github/workflows/release.yml`, triggered on `push: tags: ['v*']`, with a `verify` job that compares `${GITHUB_REF_NAME#v}` with the workspace version from `cargo metadata` and runs `devbox run check`. Verify with `actionlint` and by reading through the mismatch error path.
- [x] 2.2 Add the `build` matrix (aarch64-apple-darwin/macos-latest, x86_64-unknown-linux-musl/ubuntu-latest, aarch64-unknown-linux-musl/ubuntu-24.04-arm), using `dtolnay/rust-toolchain` pinned to 1.97.1 with a comment pointing to `devbox.json`. It runs `cargo build --release --locked`, packages `cyamus-<target>.tar.gz` with a `.sha256`, and uploads the artifacts. Verify with `actionlint`, and locally build `x86_64-unknown-linux-musl` or `aarch64-apple-darwin` and confirm the tarball holds only `cyamus` and `sha256sum -c` passes.
- [x] 2.3 Add the `publish` job (`needs: build`, `contents: write`) that creates the GitHub Release with `--generate-notes` and attaches all tarballs, checksums and `install.sh`. Verify with `actionlint`.

## 3. Documentation

- [x] 3.1 Rewrite README "Installation": curl one-liner (`curl -fsSL https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh | sh`), a table of the `CYAMUS_VERSION` and `CYAMUS_INSTALL_DIR` overrides, a "download and inspect first" variant, and the existing `cargo install` moved under "From source". Verify the documented commands match the script's variable names and URLs.
- [x] 3.2 Update README "Platform support" to list the three prebuilt targets, and that Intel macOS is supported from source only. Add a brief "Releasing" note (bump `[workspace.package].version`, commit, `git tag vX.Y.Z`, push the tag). Verify by proofreading the rendered README.

## 4. End-to-end validation

- [ ] 4.1 Push tag `v0.1.0` and confirm the release has 3 tarballs, 3 `.sha256` files and `install.sh`. Then run the documented curl one-liner on macOS arm64 and on a Linux container (x86_64; aarch64 if available), and confirm `cyamus --version` prints `cyamus 0.1.0`. Also confirm that a deliberately mismatched tag fails in `verify` and publishes nothing.
