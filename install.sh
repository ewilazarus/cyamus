#!/bin/sh
# Install a prebuilt cyamus binary.
#
#   curl -fsSL https://github.com/ewilazarus/cyamus/releases/latest/download/install.sh | sh
#
# Environment:
#   CYAMUS_VERSION      release to install, e.g. 0.2.0 or v0.2.0 (default: latest)
#   CYAMUS_INSTALL_DIR  directory to install into (default: $HOME/.local/bin)

set -eu

repo_releases="${CYAMUS_DOWNLOAD_BASE:-https://github.com/ewilazarus/cyamus/releases}"

say() {
    printf 'cyamus-install: %s\n' "$1"
}

die() {
    say "error: $1" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "required tool not found: $1"
}

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)

    # An arm64 Mac running this shell under Rosetta reports x86_64.
    if [ "$os" = Darwin ] && [ "$arch" = x86_64 ] &&
        [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
        arch=arm64
    fi

    case "$os/$arch" in
        Darwin/arm64 | Darwin/aarch64) echo aarch64-apple-darwin ;;
        Linux/x86_64 | Linux/amd64) echo x86_64-unknown-linux-musl ;;
        Linux/aarch64 | Linux/arm64) echo aarch64-unknown-linux-musl ;;
        *)
            die "no prebuilt binary for $os/$arch; install from source instead:
  cargo install --git https://github.com/ewilazarus/cyamus cyamus-cli"
            ;;
    esac
}

# CYAMUS_DOWNLOAD_BASE is a testing hook for serving releases locally; only the
# default GitHub source is restricted to HTTPS.
fetch() {
    if [ -n "${CYAMUS_DOWNLOAD_BASE:-}" ]; then
        curl -fsSL -o "$1" "$2"
    else
        curl -fsSL --proto '=https' --tlsv1.2 -o "$1" "$2"
    fi
}

sha256_check() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$1" >/dev/null 2>&1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$1" >/dev/null 2>&1
    else
        die "required tool not found: sha256sum or shasum"
    fi
}

main() {
    need curl
    need tar
    need uname
    command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 ||
        die "required tool not found: sha256sum or shasum"

    target=$(detect_target)
    asset="cyamus-$target.tar.gz"

    version="${CYAMUS_VERSION:-}"
    if [ -n "$version" ]; then
        version="v${version#v}"
        base_url="$repo_releases/download/$version"
    else
        base_url="$repo_releases/latest/download"
    fi

    install_dir="${CYAMUS_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}"

    tmp=$(mktemp -d)
    staged=""
    trap 'rm -rf "$tmp"; [ -z "$staged" ] || rm -f "$staged"' EXIT
    trap 'exit 1' HUP INT TERM

    say "downloading $asset (${version:-latest})"
    fetch "$tmp/$asset" "$base_url/$asset" ||
        die "download failed: $base_url/$asset"
    fetch "$tmp/$asset.sha256" "$base_url/$asset.sha256" ||
        die "download failed: $base_url/$asset.sha256"

    (cd "$tmp" && sha256_check "$asset.sha256") ||
        die "checksum mismatch for $asset; nothing was installed"

    tar -xzf "$tmp/$asset" -C "$tmp" cyamus || die "could not extract cyamus from $asset"

    mkdir -p "$install_dir"
    # Stage next to the destination, then rename, so a running cyamus is
    # replaced atomically instead of being overwritten in place.
    staged="$install_dir/.cyamus.$$"
    cp "$tmp/cyamus" "$staged"
    chmod 755 "$staged"
    mv -f "$staged" "$install_dir/cyamus"
    staged=""

    say "installed $("$install_dir/cyamus" --version) to $install_dir/cyamus"

    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *)
            say "$install_dir is not on your PATH; add this to your shell profile:"
            # shellcheck disable=SC2016 # print $PATH literally
            printf '\n  export PATH="%s:$PATH"\n\n' "$install_dir"
            ;;
    esac
}

main "$@"
