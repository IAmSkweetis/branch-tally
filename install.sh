#!/bin/sh
# Install branch-tally from a GitHub release.
#
#   curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
#   sh install.sh [--version vX.Y.Z] [--to DIR]
#
# Run `sh install.sh --help` for the options. POSIX sh. Never edits shell startup files.
set -eu

REPO=IAmSkweetis/branch-tally
DEFAULT_BASE_URL=https://github.com/$REPO/releases

usage() {
  cat <<'EOF'
Install branch-tally from a GitHub release.

Usage: install.sh [options]

Options:
  --version <vX.Y.Z>      Install this release instead of the latest.
  --to <dir>              Install into <dir> (default: $HOME/.local/bin).
  -h, --help              Show this help.

Environment (a flag overrides its variable):
  BRANCH_TALLY_INSTALL_DIR                 Same as --to.
  BRANCH_TALLY_INSTALL_VERSION             Same as --version.
  BRANCH_TALLY_INSTALL_BASE_URL            Release mirror (default:
                                           https://github.com/IAmSkweetis/branch-tally/releases).

The archive is verified against the release's SHA256SUMS before anything is installed. A checksum
mismatch, or an archive missing from SHA256SUMS, is a hard failure. This script does not edit your
shell startup files; if the install directory is not on PATH it prints the line to add.
EOF
}

die() {
  printf 'install.sh: error: %s\n' "$*" >&2
  exit 1
}

info() {
  printf '%s\n' "$*"
}

version=${BRANCH_TALLY_INSTALL_VERSION:-latest}
dest=${BRANCH_TALLY_INSTALL_DIR:-}
base=${BRANCH_TALLY_INSTALL_BASE_URL:-$DEFAULT_BASE_URL}
base=${base%/}

while [ $# -gt 0 ]; do
  case $1 in
    --version)
      [ $# -ge 2 ] || die "--version needs a value"
      version=$2
      shift 2
      ;;
    --to)
      [ $# -ge 2 ] || die "--to needs a value"
      dest=$2
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown option '$1' (try --help)"
      ;;
  esac
done

if [ -z "$dest" ]; then
  [ -n "${HOME:-}" ] || die "HOME is not set; pass --to <dir>"
  dest=$HOME/.local/bin
fi
dest=${dest%/}
[ -n "$dest" ] || dest=/

# Version: "latest", or vMAJOR.MINOR.PATCH (the leading v is optional on input).
if [ "$version" != latest ]; then
  case $version in
    v*) ;;
    *) version=v$version ;;
  esac
  case ${version#v} in
    *[!0-9.]* | .* | *. | *..* | '') die "version '$version' is not of the form vMAJOR.MINOR.PATCH" ;;
  esac
  dots=$(printf '%s' "${version#v}" | tr -cd . | wc -c)
  [ "$dots" -eq 2 ] || die "version '$version' is not of the form vMAJOR.MINOR.PATCH"
fi

# Platform. Linux gets the static musl build whatever the libc.
os=$(uname -s)
arch=$(uname -m)
case $os in
  Linux)
    case $arch in
      x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
      aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
      *) die "unsupported architecture '$arch' on $os" ;;
    esac
    ;;
  Darwin)
    case $arch in
      arm64 | aarch64) target=aarch64-apple-darwin ;;
      x86_64)
        # A shell running under Rosetta reports x86_64 on an Apple silicon Mac.
        if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ]; then
          target=aarch64-apple-darwin
        else
          target=x86_64-apple-darwin
        fi
        ;;
      *) die "unsupported architecture '$arch' on $os" ;;
    esac
    ;;
  MINGW* | MSYS* | CYGWIN*)
    die "this is a Windows shell ($os); install from PowerShell instead: irm https://github.com/$REPO/releases/latest/download/install.ps1 | iex"
    ;;
  *) die "unsupported operating system '$os'" ;;
esac

# Tools.
if command -v curl >/dev/null 2>&1; then
  fetch_tool=curl
elif command -v wget >/dev/null 2>&1; then
  fetch_tool=wget
else
  die "need curl or wget"
fi
if command -v sha256sum >/dev/null 2>&1; then
  sha_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
  sha_tool=shasum
else
  die "need sha256sum or shasum"
fi
command -v tar >/dev/null 2>&1 || die "need tar"

# fetch <url> <outfile>
fetch() {
  if [ "$fetch_tool" = curl ]; then
    curl -fsSL --retry 2 -o "$2" "$1" || return 1
  else
    wget -q -O "$2" "$1" || return 1
  fi
}

sha256_of() {
  if [ "$sha_tool" = sha256sum ]; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# sums_name <SHA256SUMS> <target>: the archive name listed for this target, if any.
sums_name() {
  awk -v t="-$2." '
    { n = $2; sub(/^\*/, "", n) }
    index(n, "branch-tally-v") == 1 && index(n, t) && n ~ /\.(tar\.gz|zip)$/ { print n; exit }
  ' "$1"
}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/branch-tally-install.XXXXXX") || die "cannot create a temp directory"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Resolve "latest" to a concrete tag without the GitHub API (no rate limits): the archive names
# embed the version, so read the target's name out of latest/download/SHA256SUMS. Everything after
# is fetched from download/<tag>/, never latest/, so a release landing mid-install cannot mix two
# versions: the archive and its SHA256SUMS always come from the same tag, and the checksum is
# still verified before anything is installed.
if [ "$version" = latest ]; then
  fetch "$base/latest/download/SHA256SUMS" "$tmp/latest.sums" ||
    die "could not download $base/latest/download/SHA256SUMS"
  resolved=$(sums_name "$tmp/latest.sums" "$target")
  [ -n "$resolved" ] || die "the latest release has no build for $target"
  tag=${resolved#branch-tally-}
  tag=${tag%"-$target".*}
else
  tag=$version
fi

case $target in
  *windows*) ext=zip ;;
  *) ext=tar.gz ;;
esac
archive=branch-tally-$tag-$target.$ext
url=$base/download/$tag

info "Installing branch-tally $tag for $target"
fetch "$url/SHA256SUMS" "$tmp/SHA256SUMS" || die "could not download $url/SHA256SUMS"
expected=$(awk -v a="$archive" '{ n = $2; sub(/^\*/, "", n); if (n == a) { print $1; exit } }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || die "$archive is not listed in SHA256SUMS; refusing to install"
case $expected in
  *[!0-9a-fA-F]*) die "malformed checksum for $archive in SHA256SUMS" ;;
esac
[ "${#expected}" -eq 64 ] || die "malformed checksum for $archive in SHA256SUMS"

fetch "$url/$archive" "$tmp/$archive" || die "could not download $url/$archive"
actual=$(sha256_of "$tmp/$archive")
expected=$(printf '%s' "$expected" | tr 'A-F' 'a-f')
[ "$actual" = "$expected" ] || die "checksum mismatch for $archive (expected $expected, got $actual); nothing installed"

mkdir "$tmp/x"
tar -xzf "$tmp/$archive" -C "$tmp/x" || die "could not extract $archive"
bin=$tmp/x/branch-tally-$tag-$target/branch-tally
[ -f "$bin" ] || die "$archive does not contain branch-tally"

mkdir -p "$dest" || die "cannot create $dest"
# Stage beside the destination and rename over it, so an existing binary is replaced atomically.
staged=$dest/.branch-tally.new.$$
cp "$bin" "$staged" || die "cannot write to $dest"
chmod 755 "$staged" || { rm -f "$staged"; die "cannot chmod $staged"; }
mv -f "$staged" "$dest/branch-tally" || { rm -f "$staged"; die "cannot install to $dest/branch-tally"; }

info "Installed $dest/branch-tally"
"$dest/branch-tally" --version || die "the installed binary did not run"

case ":${PATH:-}:" in
  *":$dest:"* | *":$dest/:"*) ;;
  *)
    info ""
    info "$dest is not on your PATH. Add it, for example in your shell profile:"
    info "  export PATH=\"$dest:\$PATH\""
    ;;
esac
