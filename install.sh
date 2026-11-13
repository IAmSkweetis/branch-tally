#!/bin/sh
# Install branch-tally from a GitHub release.
#
#   curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
#   sh install.sh [--version vX.Y.Z] [--to DIR] [--require-attestation]
#
# Run `sh install.sh --help` for the options. POSIX sh. Never edits shell startup files.
#
# Everything below is a function, and the script's only top-level command is the last line, so a
# download cut short can never run partway: an unterminated `main` is a syntax error.
set -eu

REPO=IAmSkweetis/branch-tally
DEFAULT_BASE_URL=https://github.com/$REPO/releases
ISSUER=https://token.actions.githubusercontent.com
# The signer must be this repository's release workflow, run from main. release.yml starts from
# workflow_run on main, and a manual dispatch on another branch is not a release.
SIGNER_WORKFLOW=$REPO/.github/workflows/release.yml
SOURCE_REF=refs/heads/main
SIGNER_IDENTITY=https://github.com/$SIGNER_WORKFLOW@$SOURCE_REF
# The gh flags that hold an attestation to that signer, for the command we run and the one we print.
GH_PINS="--repo $REPO --signer-workflow $SIGNER_WORKFLOW --source-ref $SOURCE_REF --cert-oidc-issuer $ISSUER"

usage() {
  cat <<'USAGE'
Install branch-tally from a GitHub release.

Usage: install.sh [options]

Options:
  --version <vX.Y.Z>      Install this release instead of the latest.
  --to <dir>              Install into <dir> (default: $HOME/.local/bin).
  --require-attestation   Fail unless the archive's build provenance was verified. Without it, a
                          machine with neither gh nor cosign installs with a warning.
  -h, --help              Show this help.

Environment (a flag overrides its variable):
  BRANCH_TALLY_INSTALL_DIR                 Same as --to.
  BRANCH_TALLY_INSTALL_VERSION             Same as --version.
  BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION Same as --require-attestation when set to 1, true or yes.
  BRANCH_TALLY_INSTALL_BASE_URL            Release mirror, https:// or file:// only (default:
                                           https://github.com/IAmSkweetis/branch-tally/releases).
                                           The effective source is printed when this is set.

Verification. The archive is checked against the release's SHA256SUMS before anything is
installed. That is an integrity check: it catches a corrupt or truncated download, and it does not
prove who built the archive, because the checksum file comes from the same place as the archive.
Only the build-provenance attestation proves origin. After the checksum check this script verifies
it, using the first of these that is available:

  1. gh (logged in):  gh attestation verify <archive> --repo IAmSkweetis/branch-tally \
                        --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
                        --source-ref refs/heads/main \
                        --cert-oidc-issuer https://token.actions.githubusercontent.com
  2. cosign:          verifies the release's provenance.sigstore.json with an exact
                      --certificate-identity
                      https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main
                      and --certificate-oidc-issuer https://token.actions.githubusercontent.com
  3. neither:         warns that provenance was NOT verified, prints the command to run, and
                      installs. --require-attestation turns this warning into a failure.

A verification that runs and fails is always a hard failure: nothing is installed. A checksum
mismatch, or an archive missing from SHA256SUMS, is too. This script does not edit your shell
startup files; if the install directory is not on PATH it prints the line to add.
USAGE
}

die() {
  printf 'install.sh: error: %s\n' "$*" >&2
  exit 1
}

warn() {
  printf 'install.sh: warning: %s\n' "$*" >&2
}

info() {
  printf '%s\n' "$*"
}

# printable <text>: the text with anything outside [A-Za-z0-9._/:@=+-] dropped, for echoing
# untrusted values (a tag read from the network) into an error message.
printable() {
  printf '%s' "$1" | tr -cd 'A-Za-z0-9._/:@=+-'
}

# valid_tag <tag>: true when the tag is vMAJOR.MINOR.PATCH, digits only.
valid_tag() {
  case $1 in
    v*) ;;
    *) return 1 ;;
  esac
  case ${1#v} in
    *[!0-9.]* | .* | *. | *..* | '') return 1 ;;
  esac
  dots=$(printf '%s' "${1#v}" | tr -cd . | wc -c)
  [ "$dots" -eq 2 ]
}

truthy() {
  case $1 in
    1 | true | TRUE | True | yes | YES | Yes) return 0 ;;
    *) return 1 ;;
  esac
}

# fetch <url> <outfile>: https:// is fetched with TLS 1.2+ and redirects held to HTTPS. file:// is a
# plain copy (the test and local-mirror seam); curl would refuse it under --proto '=https'.
fetch() {
  case $1 in
    file://*)
      cp "${1#file://}" "$2" || return 1
      ;;
    *)
      if [ "$fetch_tool" = curl ]; then
        curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 2 -o "$2" "$1" || return 1
      elif [ "$wget_https_only" = 1 ]; then
        wget -q --https-only -O "$2" "$1" || return 1
      else
        # This wget cannot restrict redirects. The URL itself was checked to be https://.
        wget -q -O "$2" "$1" || return 1
      fi
      ;;
  esac
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

cleanup() { rm -rf "$tmp"; }

# unverified <reason>: provenance could not be checked. A warning that installs, or a failure with
# --require-attestation.
unverified() {
  if [ "$require_attestation" = 1 ]; then
    die "build provenance could not be verified ($1) and --require-attestation is set; nothing installed. Install gh or cosign, or run: gh attestation verify $archive $GH_PINS"
  fi
  warn "provenance NOT verified: $1."
  warn "The checksum is an integrity check only; it does not prove who built this archive."
  warn "To verify it yourself, download $url/$archive and run:"
  warn "  gh attestation verify $archive $GH_PINS"
}

# verify_provenance: verify the downloaded archive's attestation. Returns normally when verified or
# when unavailable and not required; exits on a verification that ran and failed.
verify_provenance() {
  if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    # $GH_PINS is deliberately unquoted: it is a fixed list of words with no spaces inside a word.
    # shellcheck disable=SC2086
    if gh attestation verify "$tmp/$archive" $GH_PINS >"$tmp/attest.out" 2>&1; then
      info "Provenance verified with gh: $archive was built by $REPO"
      return 0
    fi
    cat "$tmp/attest.out" >&2
    die "attestation verification failed for $archive; nothing installed"
  fi
  if command -v cosign >/dev/null 2>&1; then
    if ! fetch "$url/provenance.sigstore.json" "$tmp/provenance.sigstore.json"; then
      unverified "cosign is installed but the release has no provenance.sigstore.json"
      return 0
    fi
    if cosign verify-blob-attestation --bundle "$tmp/provenance.sigstore.json" --new-bundle-format \
      --type slsaprovenance1 \
      --certificate-identity "$SIGNER_IDENTITY" \
      --certificate-oidc-issuer "$ISSUER" \
      "$tmp/$archive" >"$tmp/attest.out" 2>&1; then
      info "Provenance verified with cosign: $archive was built by $SIGNER_WORKFLOW on $SOURCE_REF"
      return 0
    fi
    cat "$tmp/attest.out" >&2
    die "attestation verification failed for $archive; nothing installed"
  fi
  unverified "neither gh (logged in) nor cosign is available"
}

main() {
  version=${BRANCH_TALLY_INSTALL_VERSION:-latest}
  dest=${BRANCH_TALLY_INSTALL_DIR:-}
  base=${BRANCH_TALLY_INSTALL_BASE_URL:-$DEFAULT_BASE_URL}
  base=${base%/}
  require_attestation=0
  if truthy "${BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION:-0}"; then require_attestation=1; fi

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
      --require-attestation)
        require_attestation=1
        shift
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

  # The release source must be https:// or file://, as for `branch-tally update`. Plain http is
  # refused: it would let anyone on the path replace both the archive and its checksum.
  case $base in
    https://* | file://*) ;;
    *) die "BRANCH_TALLY_INSTALL_BASE_URL must start with https:// or file:// (got '$(printable "$base")')" ;;
  esac
  if [ "$base" != "$DEFAULT_BASE_URL" ]; then
    info "Using release mirror: $base"
  fi

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
    valid_tag "$version" || die "version '$(printable "$version")' is not of the form vMAJOR.MINOR.PATCH"
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
  wget_https_only=0
  if command -v curl >/dev/null 2>&1; then
    fetch_tool=curl
  elif command -v wget >/dev/null 2>&1; then
    fetch_tool=wget
    if wget --help 2>&1 | grep -- '--https-only' >/dev/null 2>&1; then wget_https_only=1; fi
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

  tmp=$(mktemp -d "${TMPDIR:-/tmp}/branch-tally-install.XXXXXX") || die "cannot create a temp directory"
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
    valid_tag "$tag" || die "tag '$(printable "$tag")' read from SHA256SUMS is not of the form vMAJOR.MINOR.PATCH"
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
  info "Checksum OK (an integrity check; it does not prove who built the archive)"

  verify_provenance

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
}

main "$@"
