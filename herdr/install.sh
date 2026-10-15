#!/usr/bin/env bash
# herdr's `[[build]]` step: put this platform's binary in bin/ (`plugin link` skips it).
#
# 1. Download the release matching the manifest version and verify its checksum.
# 2. No release for this platform/version, or offline: build from source if cargo exists.
#
# GITDECK_FROM_SOURCE=1 skips the download and always builds.
set -euo pipefail

NAME="herdr-gitdeck"
REPO="pgen0x/herdr-gitdeck"
SAY="gitdeck"

# Resolve the root from this script: build commands do not get herdr's runtime env.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$ROOT/bin"
VERSION="$(grep -m1 '^version' "$ROOT/herdr-plugin.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
TAG="v${VERSION}"

# Copy then rename: replaces the binary even while a pane is still running the old one.
place() {
  mkdir -p "$BIN_DIR"
  install -m 0755 "$1" "$BIN_DIR/$NAME.new"
  mv -f "$BIN_DIR/$NAME.new" "$BIN_DIR/$NAME"
  echo "$SAY: installed $BIN_DIR/$NAME"
}

build_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "$SAY: no prebuilt binary and no cargo; install Rust from https://rustup.rs and retry" >&2
    exit 1
  fi
  echo "$SAY: building from source (cargo build --release)"
  cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"
  place "$ROOT/target/release/$NAME"
}

target=""
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Linux-aarch64 | Linux-arm64) target="aarch64-unknown-linux-musl" ;;
  Linux-x86_64) target="x86_64-unknown-linux-musl" ;;
esac

if [ -n "${GITDECK_FROM_SOURCE:-}" ] || [ -z "$target" ]; then
  build_from_source
  exit 0
fi

archive="${NAME}-${target}.tar.gz"
checksum="${NAME}-${target}.sha256"
base="https://github.com/${REPO}/releases/download/${TAG}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# GitHub's CDN can 404 a fresh release asset for a few minutes, so retry a little.
dl() {
  for _ in 1 2 3; do
    curl -fsSL "$1" -o "$2" 2>/dev/null && return 0
    sleep 2
  done
  return 1
}

echo "$SAY: downloading $archive ($TAG)"
if ! dl "$base/$archive" "$tmp/$archive" || ! dl "$base/$checksum" "$tmp/$checksum"; then
  echo "$SAY: no release asset for $TAG/$target"
  build_from_source
  exit 0
fi

expected="$(awk '{print $1}' "$tmp/$checksum")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$archive" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')"
fi
if [ "$expected" != "$actual" ]; then
  echo "$SAY: checksum mismatch for $archive (expected $expected, got $actual)" >&2
  exit 1
fi

tar -xzf "$tmp/$archive" -C "$tmp"
place "$tmp/$NAME"
