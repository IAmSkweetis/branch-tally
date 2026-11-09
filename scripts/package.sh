#!/bin/sh
# Package one built binary as a release archive:
#
#   package.sh <vX.Y.Z> <target-triple> [path/to/binary]
#
# Writes $DIST_DIR/branch-tally-<vX.Y.Z>-<target>.tar.gz (.zip for Windows targets), holding one
# directory with the binary, LICENSE and README.md. DIST_DIR defaults to ./dist. The binary
# defaults to target/<target>/release/branch-tally[.exe], where `cargo build --release --target`
# puts it. Prints the archive path. Run from anywhere: LICENSE and README.md come from the
# repository root. Used by build.yml, and runnable locally.
set -eu

usage() {
  echo "usage: package.sh <vX.Y.Z> <target-triple> [path/to/binary]" >&2
  exit 2
}

[ $# -ge 2 ] || usage
version=$1
target=$2

case $version in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "error: version '$version' is not of the form vMAJOR.MINOR.PATCH" >&2; exit 2 ;;
esac
# Reject anything but digits and dots after the v (a case glob can't say "digits only").
if printf '%s' "${version#v}" | grep -Eq '[^0-9.]|^\.|\.\.|\.$' ||
  [ "$(printf '%s' "${version#v}" | tr -cd . | wc -c)" -ne 2 ]; then
  echo "error: version '$version' is not of the form vMAJOR.MINOR.PATCH" >&2
  exit 2
fi
[ -n "$target" ] || usage

case $target in
  *windows*) ext=.exe; kind=zip ;;
  *) ext=; kind=tar ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
bin=${3:-$root/target/$target/release/branch-tally$ext}
if [ ! -f "$bin" ]; then
  echo "error: binary not found: $bin (build it first: cargo build --release --locked --target $target)" >&2
  exit 1
fi
for f in LICENSE README.md; do
  [ -f "$root/$f" ] || { echo "error: $root/$f not found" >&2; exit 1; }
done

dist=${DIST_DIR:-dist}
mkdir -p "$dist"
dist=$(cd "$dist" && pwd)
name="branch-tally-$version-$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
cp "$bin" "$stage/$name/branch-tally$ext"
chmod +x "$stage/$name/branch-tally$ext"
cp "$root/LICENSE" "$root/README.md" "$stage/$name/"

if [ "$kind" = tar ]; then
  archive="$dist/$name.tar.gz"
  rm -f "$archive"
  tar -C "$stage" -czf "$archive" "$name"
else
  archive="$dist/$name.zip"
  rm -f "$archive"
  if command -v zip >/dev/null 2>&1; then
    (cd "$stage" && zip -qr "$archive" "$name")
  elif command -v 7z >/dev/null 2>&1; then
    win=$archive; ! command -v cygpath >/dev/null 2>&1 || win=$(cygpath -w "$archive")
    (cd "$stage" && 7z a -tzip -bso0 -bsp0 "$win" "$name")
  else
    echo "error: neither zip nor 7z found; install one" >&2
    exit 1
  fi
fi
echo "$archive"
