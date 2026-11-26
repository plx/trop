#!/usr/bin/env bash
# Install the pinned Verus release used to verify trop/src/verified.
#
# Usage:
#   scripts/install-verus.sh               install (if needed) and print the bin dir
#   scripts/install-verus.sh --print-path  print the bin dir; fail if not installed
#
# The release is pinned to the weekly Verus build whose vstd matches the
# `vstd` version pinned in the workspace Cargo.toml. Bump all three together:
# VERUS_VERSION, the checksums below, and `vstd` in Cargo.toml.
#
# Environment:
#   VERUS_INSTALL_DIR  install root (default: ~/.local/share/trop-verus)
#   GITHUB_PATH        when set (GitHub Actions), the bin dir is appended to it
set -euo pipefail

VERUS_VERSION="0.2026.11.22.1ce6a59"
# Rust toolchain the verifier is built against (from the release's version.json).
VERUS_RUST_TOOLCHAIN="1.100.0"

checksum_for() {
    case "$1" in
        x86-linux) echo "c955d5958877f4f907cce80aa996d4239a481984829d82e325938c7bf242b0db" ;;
        arm64-macos) echo "8efd216b6d104bfa4fe946373fe7d260ad0db613c2c71d074744f1c91eda1e4d" ;;
        x86-macos) echo "9833442c8d402f329a1ea2cff0117e77e04009031260d6c88b8c771df1b69982" ;;
        *) return 1 ;;
    esac
}

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os/$arch" in
        Linux/x86_64) echo "x86-linux" ;;
        Darwin/arm64 | Darwin/aarch64) echo "arm64-macos" ;;
        Darwin/x86_64) echo "x86-macos" ;;
        *)
            echo "install-verus: no prebuilt Verus release for $os/$arch" >&2
            return 1
            ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

install_root="${VERUS_INSTALL_DIR:-$HOME/.local/share/trop-verus}"
install_dir="$install_root/$VERUS_VERSION"
bin_dir="$install_dir/verus"

is_installed() {
    [[ -x "$bin_dir/verus" && -x "$bin_dir/cargo-verus" && -f "$bin_dir/version.json" ]] &&
        grep -q "\"version\": \"$VERUS_VERSION\"" "$bin_dir/version.json"
}

if [[ "${1:-}" == "--print-path" ]]; then
    if is_installed; then
        echo "$bin_dir"
        exit 0
    fi
    echo "install-verus: Verus $VERUS_VERSION is not installed; run 'just install-verus'" >&2
    exit 1
fi

if ! is_installed; then
    platform="$(detect_platform)"
    expected="$(checksum_for "$platform")"
    tag="release/$VERUS_VERSION"
    url="https://github.com/verus-lang/verus/releases/download/${tag//\//%2F}/verus-$VERUS_VERSION-$platform.zip"

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    echo "install-verus: downloading Verus $VERUS_VERSION ($platform)" >&2
    curl --fail --silent --show-error --location --retry 4 --output "$tmp/verus.zip" "$url"

    actual="$(sha256_of "$tmp/verus.zip")"
    if [[ "$actual" != "$expected" ]]; then
        echo "install-verus: checksum mismatch for $url" >&2
        echo "  expected $expected" >&2
        echo "  actual   $actual" >&2
        exit 1
    fi

    mkdir -p "$tmp/unpacked"
    unzip -q "$tmp/verus.zip" -d "$tmp/unpacked"
    rm -rf "$install_dir"
    mkdir -p "$install_dir"
    mv "$tmp/unpacked/verus-$platform" "$bin_dir"
fi

if ! rustup toolchain list | grep -q "^$VERUS_RUST_TOOLCHAIN-"; then
    echo "install-verus: installing Rust $VERUS_RUST_TOOLCHAIN for the verifier" >&2
    rustup toolchain install "$VERUS_RUST_TOOLCHAIN" --profile minimal >&2
fi

if [[ -n "${GITHUB_PATH:-}" ]]; then
    echo "$bin_dir" >>"$GITHUB_PATH"
fi

echo "$bin_dir"
