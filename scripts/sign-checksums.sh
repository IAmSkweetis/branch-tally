#!/bin/sh
# Sign SHA256SUMS with minisign and prove the signature before anything is published:
#
#   sign-checksums.sh <vX.Y.Z> <dist-dir> <public-key-file>
#
# Reads the secret key from $MINISIGN_SECRET_KEY (the contents of the minisign secret key file) and
# its password from $MINISIGN_PASSWORD (must be set; empty for a key made with -W). Writes
# <dist-dir>/SHA256SUMS.minisig with the trusted comment "branch-tally <tag> SHA256SUMS", then
# verifies it against the committed public key and checks the trusted comment. Anything missing or
# wrong exits non-zero and leaves no signature behind: a release is never published unsigned.
# See docs/decisions/0004-release-provenance.md.
set -eu

usage() {
  echo "usage: sign-checksums.sh <vX.Y.Z> <dist-dir> <public-key-file>" >&2
  exit 2
}
fail() {
  echo "error: $*" >&2
  [ -z "${sig:-}" ] || rm -f "$sig"
  exit 1
}

[ $# -eq 3 ] || usage
tag=$1
dist=$2
pub=$3

case $tag in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "tag '$tag' is not of the form vMAJOR.MINOR.PATCH" ;;
esac
if printf '%s' "${tag#v}" | grep -Eq '[^0-9.]|^\.|\.\.|\.$' ||
  [ "$(printf '%s' "${tag#v}" | tr -cd . | wc -c)" -ne 2 ]; then
  fail "tag '$tag' is not of the form vMAJOR.MINOR.PATCH"
fi

sums=$dist/SHA256SUMS
sig=$dist/SHA256SUMS.minisig
rm -f "$sig"

command -v minisign >/dev/null 2>&1 ||
  fail "minisign not found; install it (scripts/ci-install-minisign.sh, or https://jedisct1.github.io/minisign/)"
[ -s "$sums" ] || fail "$sums is missing or empty"
[ -f "$pub" ] || fail "public key file $pub not found"
if grep -q 'REPLACE_WITH' "$pub" || ! sed -n 2p "$pub" | grep -Eq '^RW[A-Za-z0-9+/=]+$'; then
  fail "$pub is not a real minisign public key (still the placeholder?): commit the maintainer's public key"
fi
[ -n "${MINISIGN_SECRET_KEY:-}" ] || fail "MINISIGN_SECRET_KEY is not set or empty: refusing to publish unsigned"
[ "${MINISIGN_PASSWORD+set}" = set ] || fail "MINISIGN_PASSWORD is not set: refusing to publish unsigned"

comment="branch-tally $tag SHA256SUMS"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(umask 077 && printf '%s\n' "$MINISIGN_SECRET_KEY" >"$tmp/secret.key")
printf '%s\n' "$MINISIGN_PASSWORD" |
  minisign -S -s "$tmp/secret.key" -m "$sums" -x "$sig" -t "$comment" >/dev/null ||
  fail "minisign could not sign $sums (wrong password or malformed key?)"

[ -s "$sig" ] || fail "minisign produced no signature"
minisign -V -q -p "$pub" -m "$sums" -x "$sig" ||
  fail "the new signature does not verify against $pub: the secret key is not the committed public key's pair"
[ "$(sed -n 3p "$sig")" = "trusted comment: $comment" ] ||
  fail "trusted comment is not exactly '$comment'"
echo "signed $sums as '$comment' and verified it against $pub"
