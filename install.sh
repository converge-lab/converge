#!/bin/sh
# Install the converge CLI from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/converge-lab/converge/main/install.sh | sh
#
# Layout: the binary lives at ~/.converge/bin/converge (hooks reference it
# by absolute path; updates swap it atomically), with a ~/.local/bin
# symlink for your PATH. Uninstall: rm -rf ~/.converge and the symlink.
#
# Trust: this first download rides TLS + GitHub (like every bootstrap);
# the SHA256SUMS manifest is checked, and its minisign signature is
# verified when the `minisign` tool is available. Updates after install
# are verified by the binary itself against its embedded key.
#
# Overrides: CONVERGE_VERSION (default: latest), CONVERGE_HOME (default:
# ~/.converge), CONVERGE_DOWNLOAD_BASE (mirrors / closed contours).

set -eu

REPO="converge-lab/converge"
# The release-signing public key (minisign). Verified best-effort here;
# authoritative verification is `converge update`'s embedded copy.
PUBKEY="__MINISIGN_PUBKEY__"

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

version="${CONVERGE_VERSION:-}"
if [ -z "$version" ]; then
  version=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep '"tag_name"' | head -1 | cut -d'"' -f4) || true
  [ -n "$version" ] || die "could not resolve the latest release (set CONVERGE_VERSION)"
fi
base="${CONVERGE_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$version}"

home="${CONVERGE_HOME:-$HOME/.converge}"
bin_dir="$home/bin"
mkdir -p "$bin_dir"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "downloading converge $version ($target)…"
artifact="converge-$target.tar.gz"
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
install -m 755 "$tmp/converge" "$bin_dir/converge"

# A PATH-visible symlink, without editing anyone's shell rc.
local_bin="$HOME/.local/bin"
mkdir -p "$local_bin"
ln -sf "$bin_dir/converge" "$local_bin/converge"

say "installed $($bin_dir/converge --version 2>/dev/null || echo converge) → $bin_dir/converge"
case ":$PATH:" in
  *":$local_bin:"*) ;;
  *) say "note: $local_bin is not in your PATH — add it, or call $bin_dir/converge directly" ;;
esac
say "next: converge init"
