#!/bin/sh
# Install PowerShell (pwsh) on a CI runner from the upstream release archive, pinned by version and
# sha256 like minisign (scripts/ci-install-minisign.sh), so the installer lint and tests run on the
# same pwsh everywhere instead of whatever the runner image ships. Linux, macOS and Windows (Git
# Bash), x64 and arm64. Bump the version and every sha256 together, then mise.toml
# (`just check-pins` fails on drift):
#
#   ci-install-pwsh.sh [install-dir]      # default $RUNNER_TEMP/pwsh, else ~/.local/share/pwsh
#
# The checksums are of the archives as published upstream: compare each with the release's
# hashes.sha256 before pinning it. A mismatch fails the install; nothing is skipped. When
# $GITHUB_PATH is set, install-dir is put first on PATH for the job's later steps.
set -eu

version=7.7.0
sha256_linux_x64=c89d9c682d2c403dfe726f3df948e359675235a5bd6fab535776de69729ca0cb
sha256_linux_arm64=e59c89776eaa59790d44b3edd14277435225ab08b6e7603763101b89de83d935
sha256_osx_x64=61906bf8f407ef5cc263874c904c551d69fbb61d42dbd8f848a461ec47bf68b5
sha256_osx_arm64=76449f078f30193fce9cdf6610b77fda14f4224cafcbdf230245ec0dab9ef10d
sha256_win_x64=7c74e03b20a75b59715847439d046a6214b1863ee79b187872980f139bb4a05f
sha256_win_arm64=5dc7440cd98f0ba786a44d7db9205df5058474fd4db143caf7171e8375f34f75

dest=${1:-${RUNNER_TEMP:-$HOME/.local/share}/pwsh}

case $(uname -m) in
  x86_64 | amd64) arch=x64 ;;
  aarch64 | arm64) arch=arm64 ;;
  *) echo "error: no PowerShell build for $(uname -m)" >&2; exit 1 ;;
esac
case $(uname -s) in
  Linux) os=linux; archive="powershell-${version}-linux-${arch}.tar.gz" ;;
  Darwin) os=osx; archive="powershell-${version}-osx-${arch}.tar.gz" ;;
  MINGW* | MSYS* | CYGWIN*) os=win; archive="PowerShell-${version}-win-${arch}.zip" ;;
  *) echo "error: no PowerShell build for $(uname -s)" >&2; exit 1 ;;
esac
case ${os}_${arch} in
  linux_x64) sha256=$sha256_linux_x64 ;;
  linux_arm64) sha256=$sha256_linux_arm64 ;;
  osx_x64) sha256=$sha256_osx_x64 ;;
  osx_arm64) sha256=$sha256_osx_arm64 ;;
  win_x64) sha256=$sha256_win_x64 ;;
  win_arm64) sha256=$sha256_win_arm64 ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
  sum() { sha256sum "$1" | cut -d' ' -f1; }
else
  sum() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$work/$archive" \
  "https://github.com/PowerShell/PowerShell/releases/download/v${version}/${archive}"
[ "$(sum "$work/$archive")" = "$sha256" ] ||
  { echo "error: $archive does not match its pinned sha256" >&2; exit 1; }

rm -rf "$dest"
mkdir -p "$dest"
if [ "$os" = win ]; then
  # Git Bash has no unzip on every runner image; Windows PowerShell 5.1 always ships.
  powershell.exe -NoProfile -NonInteractive -Command \
    "Expand-Archive -LiteralPath '$(cygpath -w "$work/$archive")' -DestinationPath '$(cygpath -w "$dest")'"
  exe="$dest/pwsh.exe"
  path_entry=$(cygpath -w "$dest")
else
  tar -xzf "$work/$archive" -C "$dest"
  chmod +x "$dest/pwsh"
  exe="$dest/pwsh"
  path_entry=$dest
fi

got=$("$exe" -NoProfile -NonInteractive -Command "\$PSVersionTable.PSVersion.ToString()" | tr -d '\r')
[ "$got" = "$version" ] ||
  { echo "error: installed pwsh reports $got, want $version" >&2; exit 1; }
[ -z "${GITHUB_PATH:-}" ] || echo "$path_entry" >>"$GITHUB_PATH"
echo "pwsh $got installed in $dest"
