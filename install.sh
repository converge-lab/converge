#!/bin/sh
# Install the staw CLI from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/staw-ai/staw/main/install.sh | sh
#
# Layout: the binary lives at ~/.staw/bin/staw (hooks reference it
# by absolute path; updates swap it atomically), with a ~/.local/bin
# symlink for your PATH. Uninstall: rm -rf ~/.staw and the symlink.
#
# Trust: this first download rides TLS + GitHub (like every bootstrap);
# the SHA256SUMS manifest is checked, and its minisign signature is
# verified when the `minisign` tool is available. Updates after install
# are verified by the binary itself against its embedded key.
#
# Overrides: STAW_VERSION (default: latest), STAW_HOME (default:
# ~/.staw), STAW_DOWNLOAD_BASE (mirrors / closed contours).

set -eu

REPO="staw-ai/staw"
# The release-signing public key (minisign, key id CC7B17D2B5E41A4A).
# Verified best-effort here; authoritative verification is `staw
# update`'s embedded copy.
PUBKEY="RWRKGuS10hd7zJFgRh360sV7125wY9FsBVfrW1mCjmcEaSGwb4D3YNFn"

say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
  Linux-x86_64)   target="x86_64-unknown-linux-musl" ;;
  Linux-aarch64)  target="aarch64-unknown-linux-musl" ;;
  Darwin-x86_64)  target="x86_64-apple-darwin" ;;
  Darwin-arm64)   target="aarch64-apple-darwin" ;;
  *) die "unsupported platform: $os $arch" ;;
esac

version="${STAW_VERSION:-}"
if [ -z "$version" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep '"tag_name"' | head -1 | cut -d'"' -f4) || true
  [ -n "$version" ] || die "could not resolve the latest release (set STAW_VERSION)"
fi
base="${STAW_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$version}"

home="${STAW_HOME:-$HOME/.staw}"
bin_dir="$home/bin"
mkdir -p "$bin_dir"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "downloading staw $version ($target)…"
artifact="staw-$target.tar.gz"
curl -fsSL -o "$tmp/$artifact" "$base/$artifact"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"

# Integrity: the artifact must match the manifest.
(cd "$tmp" && grep " $artifact\$" SHA256SUMS | sha256sum -c - >/dev/null) \
  || die "checksum mismatch for $artifact"

# Authenticity, best-effort at bootstrap: verify the manifest signature
# when minisign is present and the key is baked in.
if command -v minisign >/dev/null 2>&1 && [ "${PUBKEY#__}" = "$PUBKEY" ]; then
  curl -fsSL -o "$tmp/SHA256SUMS.minisig" "$base/SHA256SUMS.minisig"
  minisign -Vm "$tmp/SHA256SUMS" -P "$PUBKEY" >/dev/null \
    || die "signature verification failed"
  say "manifest signature verified ✓"
else
  say "(signature not verified: minisign unavailable or key not baked in — checksum only)"
fi

tar -xzf "$tmp/$artifact" -C "$tmp"
install -m 755 "$tmp/staw" "$bin_dir/staw"

# A PATH-visible symlink, without editing anyone's shell rc.
local_bin="$HOME/.local/bin"
mkdir -p "$local_bin"
ln -sf "$bin_dir/staw" "$local_bin/staw"

say "installed $($bin_dir/staw --version 2>/dev/null || echo staw) → $bin_dir/staw"
case ":$PATH:" in
  *":$local_bin:"*) ;;
  *) say "note: $local_bin is not in your PATH — add it, or call $bin_dir/staw directly" ;;
esac
say "next: staw init"
