# Design

## Context

- There is one binary, `cyamus`, built from the `cyamus-cli` package. Its version comes from `[workspace.package].version`, which `clap`'s `version` attribute already exposes as `cyamus --version`.
- All dependencies are pure Rust with no C toolchain or openssl requirement, so building for musl and cross-arch targets needs only `rustup target add`.
- The code uses `std::os::unix`, so Windows builds are impossible without code changes. That is out of scope.
- `ci.yml` runs lint and test through devbox on `macos-latest` and `ubuntu-latest`. devbox pins Rust to 1.97.1.
- The repository is public at `github.com/ewilazarus/cyamus`, so release assets and `releases/latest/download/...` URLs work without authentication.

## Goals / Non-Goals

**Goals:**
- A small, readable `release.yml` and `install.sh` the maintainer fully owns, with no generated files.
- Every published release is complete (all targets, checksums and installer) or not published at all.

**Non-Goals:**
- Intel macOS, Windows, and Linux targets beyond x86_64 and aarch64.
- Homebrew, crates.io publishing, self-update, code signing or notarization.
- Automating version bumps or changelog generation.

## Decisions

### Hand-rolled workflow, not cargo-dist
The workflow is written by hand rather than generated. cargo-dist would name the installer `cyamus-cli-installer.sh` after the package, and its generated workflow must not be edited by hand. The project needs three targets and one script, which comes to about 150 lines of code we fully understand.

### Job graph
```
verify ──▶ build (matrix ×3) ──▶ publish
```
- **verify** (`ubuntu-latest`):
  - Reads the version with `cargo metadata --no-deps --format-version 1` and `jq`, rather than grepping `Cargo.toml`, so workspace inheritance is resolved correctly.
  - Compares it with `${GITHUB_REF_NAME#v}`.
  - Runs `devbox run check`, reusing the CI toolchain and scripts.
- **build** (matrix, `fail-fast: true`):

  | target | runner |
  |---|---|
  | `aarch64-apple-darwin` | `macos-latest` |
  | `x86_64-unknown-linux-musl` | `ubuntu-latest` |
  | `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` |

  Each job runs `cargo build --release --locked --target <t> -p cyamus-cli`. It then creates the tarball with `tar -C target/<t>/release -czf cyamus-<t>.tar.gz cyamus`, writes the checksum with `shasum -a 256` (available on both runner OSes), and uploads both with `actions/upload-artifact`.
- **publish** (`ubuntu-latest`, `permissions: contents: write`): downloads all artifacts, then runs `gh release create "$GITHUB_REF_NAME" --generate-notes` with the six files plus `install.sh`. Running this only after every build succeeds is what keeps releases all-or-nothing.

Alternative considered: creating the release up front and uploading from each matrix job. That is rejected because one failed build would leave a partial release.

### Toolchain: `dtolnay/rust-toolchain@master` with `toolchain: 1.97.1` in build jobs
devbox's nix-provided Rust can't easily add musl or cross targets. Pinning the same version through rustup keeps release binaries on the toolchain CI tested with. The version appears in two places (`devbox.json` and `release.yml`). A comment in the workflow points at `devbox.json`.

### Native runners, musl via rustup only
Each target builds on a runner of its own architecture, so no `cross` or Docker is needed. Pure-Rust musl builds link with Rust's bundled self-contained musl CRT, so `musl-tools` isn't strictly needed. The build job installs it anyway (`apt-get install musl-tools`) to stay robust if a future dependency pulls in `cc`.

### Release-hosted installer
`install.sh` is attached to every release, and the README points at `releases/latest/download/install.sh`.
- Compared with `raw.githubusercontent.com/.../main/install.sh`, a script can't change on `main` ahead of the binaries it expects, and the asset name stays stable.
- The script resolves "latest" through the `releases/latest/download/<asset>` redirect, so it never calls the GitHub API and isn't subject to API rate limits.
- A pinned version uses `releases/download/v<ver>/<asset>`.

### Installer shape
- `set -eu`.
- Downloads go into `mktemp -d`, with `trap 'rm -rf "$tmp"' EXIT`.
- Downloads use `curl -fsSL --proto '=https' --tlsv1.2`.
- Checksums are verified with `sha256sum -c` or `shasum -a 256 -c`, whichever is present, run inside the temp directory.
- The installer then extracts the archive, runs `install -m 755` into a temp name inside the target directory, and moves it into place with `mv`. Writing to a temp name first avoids "text file busy" when replacing a binary that is currently running.
- Platform detection uses `uname -s` and `uname -m`.
- On macOS under Rosetta, `uname -m` reports `x86_64`. The installer checks `sysctl -n sysctl.proc_translated` and treats the host as arm64 when it returns `1`.

### README structure
```
## Installation
  curl one-liner
  Overrides: CYAMUS_VERSION, CYAMUS_INSTALL_DIR (short table)
  ### From source      ← existing cargo install line
  devbox paragraph     ← unchanged
## Platform support    ← list the three prebuilt targets; Intel Mac via source
```
Add a short "Releasing" note: bump the version, commit, run `git tag vX.Y.Z`, then push the tag. It can go in the README or a contributor section, kept brief.

## Risks / Trade-offs

- [Toolchain version duplicated between devbox.json and release.yml] → A comment cross-references the two. The verify job runs tests through devbox, so drift shows up as a build difference, not a correctness hole.
- [Piping curl into sh is a trust decision for users] → The script is short and plain POSIX sh, and the README shows how to download and read it first. Checksums protect against corrupted downloads but not a compromised repository, which is acceptable for now.
- [`ubuntu-24.04-arm` runner availability or labels change] → It's a single matrix entry, easy to switch to `cross` on x86 if needed.
- [Tag pushed before the version bump commit] → The verify job fails fast, no release is created, and the fix is to delete the tag, then re-tag the bump commit.
- [Intel Mac users get an error] → The error message points to `cargo install --git https://github.com/ewilazarus/cyamus cyamus-cli`.

## Migration Plan

There's nothing to migrate. After merging, cut `v0.1.0` (the current version) as the first release to validate the pipeline end to end. If it fails, delete the tag or draft release and re-push. `ci.yml` is untouched.
