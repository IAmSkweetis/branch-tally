#!/bin/sh
# Install minisign on a Linux CI runner from the upstream release archive, pinned by version and
# sha256 like the repository's other downloaded tools (see gitleaks in ci.yml). Bump both together:
#
#   ci-install-minisign.sh [install-dir]      # default /usr/local/bin, via sudo when not writable
#
# The checksum below is of minisign-<version>-linux.tar.gz as published upstream. Verify a new one
# against the release's .minisig with upstream's key
# (RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3) before pinning it. A mismatch fails
# the install; nothing is skipped.
set -eu

version=0.12
sha256=9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73
dest=${1:-/usr/local/bin}

case $(uname -m) in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) echo "error: no minisign build for $(uname -m)" >&2; exit 1 ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$work/minisign.tar.gz" \
  "https://github.com/jedisct1/minisign/releases/download/${version}/minisign-${version}-linux.tar.gz"
echo "${sha256}  $work/minisign.tar.gz" | sha256sum -c - >/dev/null ||
  { echo "error: minisign ${version} archive does not match its pinned sha256" >&2; exit 1; }
tar -xzf "$work/minisign.tar.gz" -C "$work" "minisign-linux/${arch}/minisign"
if [ -w "$dest" ]; then
  install "$work/minisign-linux/${arch}/minisign" "$dest/minisign"
else
  sudo install "$work/minisign-linux/${arch}/minisign" "$dest/minisign"
fi
minisign -v | head -n1
