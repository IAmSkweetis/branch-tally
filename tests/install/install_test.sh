#!/bin/sh
# Tests for install.sh. POSIX sh, no arguments; exit 0 means every case passed.
#
# Builds a fake release tree under a temp dir and points BRANCH_TALLY_INSTALL_BASE_URL at it over
# file://. HOME and TMPDIR are redirected into the temp dir and `uname`/`sysctl` are shadowed on
# PATH, so a run never touches the real ~/.local/bin, the network or the host's platform.
#
# Run it under another shell with: TEST_SHELL="busybox sh" sh tests/install/install_test.sh
set -u

here=$(cd "$(dirname "$0")" && pwd)
installer=$here/../../install.sh
shell_cmd=${TEST_SHELL:-sh}

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT INT TERM

cases=0
failures=0

pass() { cases=$((cases + 1)); printf 'ok   %s\n' "$1"; }
fail() { cases=$((cases + 1)); failures=$((failures + 1)); printf 'FAIL %s\n' "$1"; }

# check <description> <command...>: passes when the command succeeds.
check() {
  desc=$1
  shift
  if "$@"; then pass "$desc"; else fail "$desc"; fi
}

contains() { grep -F -- "$2" "$1" >/dev/null 2>&1; }
lacks() { ! contains "$1" "$2"; }
empty_dir() { [ -z "$(ls -A "$1")" ]; }
runs_as() { [ "$("$1")" = "$2" ]; }

real_uname=$(command -v uname)
sha_cmd=sha256sum
command -v sha256sum >/dev/null 2>&1 || sha_cmd="shasum -a 256"

# The shell under test as one command name: a wrapper that execs TEST_SHELL (which may be several
# words, such as "busybox sh"), so every call site quotes it.
shell_under_test=$T/shell-under-test
printf '#!/bin/sh\nexec %s "$@"\n' "$shell_cmd" > "$shell_under_test"
chmod +x "$shell_under_test"

# A PATH of symlinks to the stock tools the installer needs, so the host's own gh, cosign and curl
# can never leak into a case. TOOLBIN has curl; TOOLBIN_NOCURL has neither curl nor wget.
TOOLBIN=$T/toolbin
TOOLBIN_NOCURL=$T/toolbin-nocurl
mkdir -p "$TOOLBIN" "$TOOLBIN_NOCURL"
for tool in sh dash bash ksh busybox awk sed tr wc cat mktemp rm cp mv chmod mkdir tar grep ls env head dirname \
  sha256sum shasum gzip basename sleep; do
  src=$(command -v "$tool" 2>/dev/null) || continue
  case $src in /*) ;; *) continue ;; esac
  ln -sf "$src" "$TOOLBIN/$tool"
  ln -sf "$src" "$TOOLBIN_NOCURL/$tool"
done
if src=$(command -v curl 2>/dev/null); then ln -sf "$src" "$TOOLBIN/curl"; fi

FAKEBIN=$T/fakebin
mkdir -p "$FAKEBIN"
cat > "$FAKEBIN/uname" <<UNAME
#!/bin/sh
case \$1 in
  -s) echo "\${FAKE_UNAME_S:-Linux}" ;;
  -m) echo "\${FAKE_UNAME_M:-x86_64}" ;;
  *) exec "$real_uname" "\$@" ;;
esac
UNAME
cat > "$FAKEBIN/sysctl" <<'SYSCTL'
#!/bin/sh
echo "${FAKE_PROC_TRANSLATED:-0}"
SYSCTL
chmod +x "$FAKEBIN/uname" "$FAKEBIN/sysctl"

# mk_release <root> <tag> <target> <stub-version>: one release under <root>/download/<tag>, with the
# archive layout scripts/package.sh produces. Does not touch latest/.
mk_release() {
  root=$1 tag=$2 target=$3 ver=$4
  name=branch-tally-$tag-$target
  stage=$T/stage
  rm -rf "$stage"
  mkdir -p "$stage/$name" "$root/download/$tag"
  printf '#!/bin/sh\necho "branch-tally %s"\n' "$ver" > "$stage/$name/branch-tally"
  chmod +x "$stage/$name/branch-tally"
  echo LICENSE > "$stage/$name/LICENSE"
  echo README > "$stage/$name/README.md"
  echo '{"fake":"bundle"}' > "$root/download/$tag/provenance.sigstore.json"
  tar -C "$stage" -czf "$root/download/$tag/$name.tar.gz" "$name"
  (cd "$root/download/$tag" && $sha_cmd "$name.tar.gz" >> SHA256SUMS)
  rm -rf "$stage"
}

# mk_latest <root> <tag>: make <tag> the latest release.
mk_latest() {
  rm -rf "$1/latest"
  mkdir -p "$1/latest"
  cp -R "$1/download/$2" "$1/latest/download"
  mk_api "$2"
}

# mk_api <tag>: what the fake GitHub API answers for releases/latest, a pretty-printed subset of the
# real response. The installers read it from file://$S/api (see run_install), or from the https
# hosts the curl stub serves out of the same tree.
mk_api() {
  mkdir -p "$S/api/repos/IAmSkweetis/branch-tally/releases"
  cat > "$S/api/repos/IAmSkweetis/branch-tally/releases/latest" <<API
{
  "url": "https://api.github.com/repos/IAmSkweetis/branch-tally/releases/1",
  "tag_name": "$1",
  "name": "$1",
  "draft": false,
  "assets": []
}
API
}

# set_api <body>: the fake API answers with exactly this text.
set_api() {
  mkdir -p "$S/api/repos/IAmSkweetis/branch-tally/releases"
  printf '%s' "$1" > "$S/api/repos/IAmSkweetis/branch-tally/releases/latest"
}

# break_sums <root> <tag>: corrupt every hash in the release's SHA256SUMS.
break_sums() {
  sed "s/^[0-9a-f]\{64\}/0000000000000000000000000000000000000000000000000000000000000000/" \
    "$1/download/$2/SHA256SUMS" > "$T/sums"
  cp "$T/sums" "$1/download/$2/SHA256SUMS"
}

# fresh <name>: a new sandbox with HOME, TMPDIR and a release root; sets $S, $R, $D (install dir).
fresh() {
  S=$T/$1
  R=$S/release
  D=$S/dest
  mkdir -p "$S/home" "$S/tmp" "$R" "$S/stubs"
  : > "$S/gh.log"
  : > "$S/cosign.log"
  : > "$S/curl.log"
  : > "$S/wget.log"
  : > "$S/curl.hdr"
  : > "$S/curl.urls"
}

# The signer and source ref the fake attestations were really made with.
GOOD_WORKFLOW=IAmSkweetis/branch-tally/.github/workflows/release.yml
GOOD_REF=refs/heads/main
GOOD_IDENTITY=https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main

# mk_gh <auth rc> <verify rc> [signer workflow] [source ref]: a gh stub that logs its arguments, one
# per line, to $S/gh.log. The fake attestation was signed by the given workflow on the given ref
# (default: release.yml on main). Like the real gh, it enforces a signer or ref only when the
# matching flag is passed: with no --signer-workflow any signer is accepted, which is the hole the
# installer must close.
mk_gh() {
  actual_wf=${3:-$GOOD_WORKFLOW}
  actual_ref=${4:-$GOOD_REF}
  cat > "$S/stubs/gh" <<GH
#!/bin/sh
for a in "\$@"; do printf '%s\n' "\$a" >> "$S/gh.log"; done
cmd="\$1 \$2"
signer= ref= issuer= repo= file=
if [ "\$cmd" = "attestation verify" ]; then
  shift 2
  while [ \$# -gt 0 ]; do
    case \$1 in
      --signer-workflow) signer=\$2; shift ;;
      --source-ref) ref=\$2; shift ;;
      --cert-oidc-issuer) issuer=\$2; shift ;;
      --repo) repo=\$2; shift ;;
      -*) exit 2 ;;
      *) file=\$1 ;;
    esac
    shift
  done
fi
case "\$cmd" in
  "auth status") exit $1 ;;
  "attestation verify")
    # gh attestation verify <file-path> --repo <owner/repo>: the archive must exist and --repo is
    # mandatory (without --repo or --owner the real gh refuses).
    [ -f "\$file" ] || exit 2
    [ "\$repo" = IAmSkweetis/branch-tally ] || exit 2
    [ $2 -eq 0 ] || exit $2
    [ -z "\$signer" ] || [ "\$signer" = "$actual_wf" ] || exit 1
    [ -z "\$ref" ] || [ "\$ref" = "$actual_ref" ] || exit 1
    [ -z "\$issuer" ] || [ "\$issuer" = https://token.actions.githubusercontent.com ] || exit 1
    exit 0 ;;
esac
exit 99
GH
  chmod +x "$S/stubs/gh"
}

# mk_cosign <rc> [signer identity]: a cosign stub that logs its arguments, one per line, to
# $S/cosign.log. Its fake bundle was signed by the given identity (default: release.yml on main).
# Verification passes only with an exact --certificate-identity equal to it and the Actions issuer;
# a regexp, or no identity, is refused.
mk_cosign() {
  actual_id=${2:-$GOOD_IDENTITY}
  cat > "$S/stubs/cosign" <<COSIGN
#!/bin/sh
for a in "\$@"; do printf '%s\n' "\$a" >> "$S/cosign.log"; done
# The real interface: cosign verify-blob-attestation --bundle <provenance.sigstore.json>
# --new-bundle-format --type slsaprovenance1 --certificate-identity <id> --certificate-oidc-issuer
# <issuer> <blob>. A missing or different piece fails here as it would in cosign.
[ "\$1" = verify-blob-attestation ] || exit 2
shift
id= issuer= bundle= type= newfmt=0 blob=
while [ \$# -gt 0 ]; do
  case \$1 in
    --certificate-identity) id=\$2; shift ;;
    --certificate-oidc-issuer) issuer=\$2; shift ;;
    --bundle) bundle=\$2; shift ;;
    --type) type=\$2; shift ;;
    --new-bundle-format) newfmt=1 ;;
    -*) exit 2 ;;
    *) blob=\$1 ;;
  esac
  shift
done
[ -f "\$bundle" ] && [ "\$(basename "\$bundle")" = provenance.sigstore.json ] || exit 2
[ "\$newfmt" = 1 ] || exit 2
[ "\$type" = slsaprovenance1 ] || exit 2
[ -f "\$blob" ] || exit 2
[ $1 -eq 0 ] || exit $1
[ "\$id" = "$actual_id" ] || exit 1
[ "\$issuer" = https://token.actions.githubusercontent.com ] || exit 1
exit 0
COSIGN
  chmod +x "$S/stubs/cosign"
}

# mk_curl: a curl stub standing in for the network. It logs its arguments to $S/curl.log and serves
# https://mirror.example.test/... from $R.
mk_curl() {
  cat > "$S/stubs/curl" <<CURL
#!/bin/sh
out= url=
while [ \$# -gt 0 ]; do
  printf '%s\n' "\$1" >> "$S/curl.log"
  case \$1 in
    -o) out=\$2; shift ;;
    -H)
      case \$2 in @*) cat "\${2#@}" >> "$S/curl.hdr" ;; *) printf '%s\n' "\$2" >> "$S/curl.hdr" ;; esac
      shift ;;
    http*) url=\$1 ;;
  esac
  shift
done
case \$url in
  https://api.github.com/* | https://api.example.test/*)
    printf '%s\n' "\$url" >> "$S/curl.urls"
    p=\${url#https://*/}
    cp "$S/api/\$p" "\$out" ;;
  *) cp "$R/\${url#https://mirror.example.test/}" "\$out" ;;
esac
CURL
  chmod +x "$S/stubs/curl"
}

# mk_wget: like mk_curl, for wget. Advertises --https-only.
mk_wget() {
  cat > "$S/stubs/wget" <<WGET
#!/bin/sh
if [ "\${1:-}" = --help ]; then echo "  --https-only  only follow secure HTTPS links"; exit 0; fi
out= url=
while [ \$# -gt 0 ]; do
  printf '%s\n' "\$1" >> "$S/wget.log"
  case \$1 in
    -O) out=\$2; shift ;;
    http*) url=\$1 ;;
  esac
  shift
done
cp "$R/\${url#https://mirror.example.test/}" "\$out"
WGET
  chmod +x "$S/stubs/wget"
}

# run_install <env assignments and command...>: runs in the sandbox; sets $rc, $out, $err.
run_install() {
  out=$S/out
  err=$S/err
  env -i PATH="$S/stubs:$FAKEBIN:${TOOLS:-$TOOLBIN}" HOME="$S/home" TMPDIR="$S/tmp" \
    FAKE_UNAME_S="${FAKE_UNAME_S:-Linux}" FAKE_UNAME_M="${FAKE_UNAME_M:-x86_64}" \
    FAKE_PROC_TRANSLATED="${FAKE_PROC_TRANSLATED:-0}" \
    BRANCH_TALLY_INSTALL_BASE_URL="file://$R" BRANCH_TALLY_INSTALL_API_URL="file://$S/api" "$@" >"$out" 2>"$err" </dev/null
  rc=$?
}

# Platform fakes are plain shell variables (not per-command assignments, which a function call may
# leave set in some shells, such as macOS's bash 3.2 running as sh). plat resets them afterwards.
FAKE_UNAME_S=Linux
FAKE_UNAME_M=x86_64
FAKE_PROC_TRANSLATED=0

X=x86_64-unknown-linux-musl
# sh_run <installer args...>
sh_run() { run_install "$shell_under_test" "$installer" "$@"; }
# plat <uname -s> <uname -m> <proc_translated> <installer args...>: sh_run on a faked platform.
plat() {
  FAKE_UNAME_S=$1 FAKE_UNAME_M=$2 FAKE_PROC_TRANSLATED=$3
  shift 3
  sh_run "$@"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 FAKE_PROC_TRANSLATED=0
}

# 1. Happy path: latest, custom dir.
fresh happy
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "happy path exits 0" test "$rc" -eq 0
check "happy path installs an executable" test -x "$D/branch-tally"
check "happy path prints the installed path" contains "$out" "$D/branch-tally"
check "happy path prints the version" contains "$out" "branch-tally 2.0.0"
check "happy path leaves no temp files in TMPDIR" empty_dir "$S/tmp"
check "happy path leaves only the binary in the dir" test "$(ls -A "$D")" = branch-tally

# 2. Pinned version through the environment, while latest is newer.
fresh pinned
mk_release "$R" v1.0.0 $X 1.0.0
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
run_install BRANCH_TALLY_INSTALL_DIR="$D" BRANCH_TALLY_INSTALL_VERSION=v1.0.0 \
  "$shell_under_test" "$installer"
check "env-pinned version exits 0" test "$rc" -eq 0
check "env-pinned version installs v1.0.0" contains "$out" "branch-tally 1.0.0"

# 2b. Flags override env.
run_install BRANCH_TALLY_INSTALL_DIR="$S/ignored" BRANCH_TALLY_INSTALL_VERSION=v2.0.0 \
  "$shell_under_test" "$installer" --version v1.0.0 --to "$D"
check "flag --version overrides env" contains "$out" "branch-tally 1.0.0"
check "flag --to overrides env" test ! -e "$S/ignored"

# 2c. A version without the leading v is accepted.
sh_run --version 1.0.0 --to "$D"
check "version without v is accepted" test "$rc" -eq 0

# 2d. A pinned version that does not exist fails.
sh_run --version v9.9.9 --to "$D"
check "unknown version exits non-zero" test "$rc" -ne 0

# 2e. A malformed version is rejected before any download.
sh_run --version 'v1;rm' --to "$D"
check "malformed version exits non-zero" test "$rc" -ne 0
check "malformed version says so" contains "$err" "version"

# 3. Checksum mismatch: hard fail, nothing installed.
fresh badsum
mk_release "$R" v2.0.0 $X 2.0.0
break_sums "$R" v2.0.0
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "checksum mismatch exits non-zero" test "$rc" -ne 0
check "checksum mismatch installs nothing" test ! -e "$D/branch-tally"
check "checksum mismatch says so on stderr" contains "$err" "checksum"
check "checksum mismatch leaves no temp files" empty_dir "$S/tmp"

# 3b. Checksum mismatch must not replace an existing install.
fresh keepold
mk_release "$R" v1.0.0 $X 1.0.0
mk_latest "$R" v1.0.0
sh_run --to "$D"
mk_release "$R" v3.0.0 $X 3.0.0
break_sums "$R" v3.0.0
mk_latest "$R" v3.0.0
sh_run --to "$D"
check "failed upgrade exits non-zero" test "$rc" -ne 0
check "failed upgrade keeps the old binary" runs_as "$D/branch-tally" "branch-tally 1.0.0"

# 4. Target missing from SHA256SUMS.
fresh notarget
mk_release "$R" v2.0.0 aarch64-apple-darwin 2.0.0
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "target missing from SHA256SUMS exits non-zero" test "$rc" -ne 0
check "target missing installs nothing" test ! -e "$D/branch-tally"
sh_run --version v2.0.0 --to "$D"
check "pinned target missing exits non-zero" test "$rc" -ne 0

# 4b. Archive present but SHA256SUMS has no line for it.
fresh nosumline
mk_release "$R" v2.0.0 $X 2.0.0
: > "$R/download/v2.0.0/SHA256SUMS"
sh_run --version v2.0.0 --to "$D"
check "missing SHA256SUMS line exits non-zero" test "$rc" -ne 0
check "missing SHA256SUMS line installs nothing" test ! -e "$D/branch-tally"

# 4c. No SHA256SUMS file at all.
rm "$R/download/v2.0.0/SHA256SUMS"
sh_run --version v2.0.0 --to "$D"
check "missing SHA256SUMS file exits non-zero" test "$rc" -ne 0

# 5. PATH hint.
fresh pathhint
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "PATH hint shown when dir is not on PATH" contains "$out" "export PATH=\"$D:\$PATH\""
run_install PATH="$D:$FAKEBIN:$TOOLBIN" "$shell_under_test" "$installer" --to "$D"
check "no PATH hint when dir is on PATH" lacks "$out" "export PATH"
run_install PATH="$D/:$FAKEBIN:$TOOLBIN" "$shell_under_test" "$installer" --to "$D"
check "no PATH hint when PATH has a trailing-slash form" lacks "$out" "export PATH"
check "installer does not write into HOME (no rc edits)" empty_dir "$S/home"

# 6. Re-install overwrites.
fresh reinstall
mk_release "$R" v1.0.0 $X 1.0.0
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v1.0.0
sh_run --to "$D"
check "first install is v1" contains "$out" "branch-tally 1.0.0"
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "re-install exits 0" test "$rc" -eq 0
check "re-install overwrites with v2" runs_as "$D/branch-tally" "branch-tally 2.0.0"

# 7. Default directory is $HOME/.local/bin and is created.
fresh defaultdir
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
sh_run
check "default dir is HOME/.local/bin" test -x "$S/home/.local/bin/branch-tally"

# 8. Unsupported OS and architecture.
fresh badplat
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
plat FreeBSD x86_64 0 --to "$D"
check "unsupported OS exits non-zero" test "$rc" -ne 0
check "unsupported OS names the OS" contains "$err" "FreeBSD"
plat Linux riscv64 0 --to "$D"
check "unsupported arch exits non-zero" test "$rc" -ne 0
check "unsupported arch names the arch" contains "$err" "riscv64"

# 8b. Git Bash, MSYS and Cygwin report a Windows uname: point at install.ps1.
for win_uname in MINGW64_NT-10.0-26100 MSYS_NT-10.0-26100 CYGWIN_NT-10.0-26100; do
  plat "$win_uname" x86_64 0 --to "$D"
  check "$win_uname exits non-zero" test "$rc" -ne 0
  check "$win_uname points at install.ps1" contains "$err" "install.ps1"
  check "$win_uname gives the PowerShell one-liner" contains "$err" "irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex"
  check "$win_uname installs nothing" test ! -e "$D/branch-tally"
done

# 9. Platform mapping: Linux arm64, macOS arm64, macOS Intel under Rosetta, native macOS Intel.
fresh plat
while read -r p_os p_arch p_translated p_target; do
  rm -rf "$R" "$D"
  mkdir -p "$R"
  mk_release "$R" v2.0.0 "$p_target" 2.0.0
  mk_latest "$R" v2.0.0
  plat "$p_os" "$p_arch" "$p_translated" --to "$D"
  check "platform $p_os/$p_arch (translated=$p_translated) selects $p_target" test "$rc" -eq 0
done <<'SPECS'
Linux aarch64 0 aarch64-unknown-linux-musl
Darwin arm64 0 aarch64-apple-darwin
Darwin x86_64 1 aarch64-apple-darwin
Darwin x86_64 0 x86_64-apple-darwin
SPECS

# 10. There is no attestation option: the installer needs nothing but stock tools.
fresh noattest
sh_run --verify-attestation --to "$D"
check "--verify-attestation is not an option" test "$rc" -ne 0

# 11. Help and bad flags.
fresh help
sh_run --help
check "--help exits 0" test "$rc" -eq 0
check "--help documents --to" contains "$out" "--to"
sh_run --bogus
check "unknown flag exits non-zero" test "$rc" -ne 0

# 12. Provenance. The checksum is an integrity check; only the attestation proves origin.
# 12a. gh logged in and the attestation verifies: installs, and gh was pinned to this repository.
fresh gh_pass
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 0 0
sh_run --to "$D"
check "gh pass exits 0" test "$rc" -eq 0
check "gh pass installs" test -x "$D/branch-tally"
check "gh pass ran attestation verify" contains "$S/gh.log" "attestation"
check "gh pass pins the repository" contains "$S/gh.log" "IAmSkweetis/branch-tally"
check "gh pass uses --repo" contains "$S/gh.log" "--repo"
check "gh pass pins the signer workflow flag" contains "$S/gh.log" "--signer-workflow"
check "gh pass pins the signer workflow to release.yml" contains "$S/gh.log" "$GOOD_WORKFLOW"
check "gh pass pins the source ref flag" contains "$S/gh.log" "--source-ref"
check "gh pass pins the source ref to main" contains "$S/gh.log" "refs/heads/main"
check "gh pass pins the issuer" contains "$S/gh.log" "https://token.actions.githubusercontent.com"
check "gh pass verifies the archive" contains "$S/gh.log" "branch-tally-v2.0.0-$X.tar.gz"
check "gh pass says provenance was verified" contains "$out" "Provenance verified"

# 12b. gh logged in and verification fails: hard failure, nothing installed, no cosign fallback.
fresh gh_fail
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 0 1
mk_cosign 0
sh_run --to "$D"
check "gh fail exits non-zero" test "$rc" -ne 0
check "gh fail installs nothing" test ! -e "$D/branch-tally"
check "gh fail says attestation" contains "$err" "attestation"
check "gh fail does not fall back to cosign" test ! -s "$S/cosign.log"
check "gh fail leaves no temp files" empty_dir "$S/tmp"

# 12c. A checksum failure stops before any provenance tool runs.
fresh gh_after_sum
mk_release "$R" v2.0.0 $X 2.0.0
break_sums "$R" v2.0.0
mk_latest "$R" v2.0.0
mk_gh 0 0
sh_run --to "$D"
check "bad checksum exits non-zero" test "$rc" -ne 0
check "bad checksum never reaches gh attestation" lacks "$S/gh.log" "attestation"

# 12d. gh not logged in, cosign present and verifies: the bundle is checked against this repository's
# release workflow and the GitHub Actions issuer.
fresh cosign_pass
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 1 0
mk_cosign 0
sh_run --to "$D"
check "cosign pass exits 0" test "$rc" -eq 0
check "cosign pass installs" test -x "$D/branch-tally"
check "cosign pass did not run gh attestation verify" lacks "$S/gh.log" "attestation"
check "cosign pass uses the bundle" contains "$S/cosign.log" "provenance.sigstore.json"
check "cosign pass pins an exact identity" contains "$S/cosign.log" "--certificate-identity"
check "cosign pass pins release.yml on main" contains "$S/cosign.log" "$GOOD_IDENTITY"
check "cosign pass uses no identity regexp" lacks "$S/cosign.log" "--certificate-identity-regexp"
check "cosign pass pins the issuer" contains "$S/cosign.log" "https://token.actions.githubusercontent.com"
check "cosign pass verifies the archive" contains "$S/cosign.log" "branch-tally-v2.0.0-$X.tar.gz"
check "cosign pass says provenance was verified" contains "$out" "Provenance verified"

# 12e. cosign verification fails: hard failure.
fresh cosign_fail
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_cosign 1
sh_run --to "$D"
check "cosign fail exits non-zero" test "$rc" -ne 0
check "cosign fail installs nothing" test ! -e "$D/branch-tally"
check "cosign fail says attestation" contains "$err" "attestation"

# 12f. cosign present but the release has no bundle: provenance could not be checked.
fresh cosign_nobundle
mk_release "$R" v2.0.0 $X 2.0.0
rm "$R/download/v2.0.0/provenance.sigstore.json"
mk_latest "$R" v2.0.0
mk_cosign 0
sh_run --to "$D"
check "no bundle installs with a warning" test "$rc" -eq 0
check "no bundle warns provenance was not verified" contains "$err" "NOT verified"
check "no bundle never ran cosign" test ! -s "$S/cosign.log"
rm -rf "$D"
sh_run --require-attestation --to "$D"
check "no bundle with --require-attestation fails" test "$rc" -ne 0

# 12g. Neither tool: warn plainly, print the command, install. Strict mode fails instead.
fresh unavailable
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "unavailable exits 0" test "$rc" -eq 0
check "unavailable installs" test -x "$D/branch-tally"
check "unavailable warns provenance was not verified" contains "$err" "NOT verified"
check "unavailable says the checksum is only an integrity check" contains "$err" "integrity"
check "unavailable prints the gh command" contains "$err" "gh attestation verify"
check "unavailable prints the repository" contains "$err" "--repo IAmSkweetis/branch-tally"
rm -rf "$D"
sh_run --require-attestation --to "$D"
check "--require-attestation with no tool exits non-zero" test "$rc" -ne 0
check "--require-attestation with no tool installs nothing" test ! -e "$D/branch-tally"
check "--require-attestation says why" contains "$err" "attestation"
run_install BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=1 "$shell_under_test" "$installer" --to "$D"
check "BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=1 exits non-zero" test "$rc" -ne 0
check "BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=1 installs nothing" test ! -e "$D/branch-tally"
run_install BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=0 "$shell_under_test" "$installer" --to "$D"
check "BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=0 does not require it" test "$rc" -eq 0

# 12h. Strict mode with a tool that verifies succeeds.
fresh strict_ok
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 0 0
sh_run --require-attestation --to "$D"
check "--require-attestation with a passing gh exits 0" test "$rc" -eq 0

# 12i. gh on PATH but not logged in, no cosign: unavailable.
fresh gh_loggedout
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 1 0
sh_run --require-attestation --to "$D"
check "logged-out gh with no cosign is unavailable (strict fails)" test "$rc" -ne 0
check "logged-out gh never ran attestation verify" lacks "$S/gh.log" "attestation"

# 12j. An attestation from another workflow, or another ref, in this repository must not install.
fresh gh_other_wf
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 0 0 IAmSkweetis/branch-tally/.github/workflows/other.yml refs/heads/main
sh_run --to "$D"
check "gh: attestation from another workflow exits non-zero" test "$rc" -ne 0
check "gh: attestation from another workflow installs nothing" test ! -e "$D/branch-tally"

fresh gh_other_ref
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_gh 0 0 "$GOOD_WORKFLOW" refs/heads/some-branch
sh_run --to "$D"
check "gh: attestation from another ref exits non-zero" test "$rc" -ne 0
check "gh: attestation from another ref installs nothing" test ! -e "$D/branch-tally"

fresh cosign_other_wf
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_cosign 0 https://github.com/IAmSkweetis/branch-tally/.github/workflows/other.yml@refs/heads/main
sh_run --to "$D"
check "cosign: bundle from another workflow exits non-zero" test "$rc" -ne 0
check "cosign: bundle from another workflow installs nothing" test ! -e "$D/branch-tally"

fresh cosign_other_ref
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_cosign 0 https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/feature
sh_run --to "$D"
check "cosign: bundle from another ref exits non-zero" test "$rc" -ne 0
check "cosign: bundle from another ref installs nothing" test ! -e "$D/branch-tally"

# 12k. The manual command, in the warning and in --help, carries the same pins.
fresh manual_cmd
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
sh_run --to "$D"
check "warning command pins the signer workflow" contains "$err" "--signer-workflow $GOOD_WORKFLOW"
check "warning command pins the source ref" contains "$err" "--source-ref refs/heads/main"
check "warning command pins the issuer" contains "$err" "--cert-oidc-issuer https://token.actions.githubusercontent.com"
sh_run --require-attestation --to "$D"
check "strict failure command pins the signer workflow" contains "$err" "--signer-workflow $GOOD_WORKFLOW"
check "strict failure command pins the source ref" contains "$err" "--source-ref refs/heads/main"
sh_run --help
check "--help command pins the signer workflow" contains "$out" "--signer-workflow $GOOD_WORKFLOW"
check "--help command pins the source ref" contains "$out" "--source-ref refs/heads/main"
check "--help names the exact cosign identity" contains "$out" "$GOOD_IDENTITY"

# 13. F-06: mirror URL scheme.
fresh scheme
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
for bad in http://mirror.example.test ftp://mirror.example.test mirror.example.test; do
  run_install BRANCH_TALLY_INSTALL_BASE_URL="$bad" "$shell_under_test" "$installer" --to "$D"
  check "mirror '$bad' is rejected" test "$rc" -ne 0
  check "mirror '$bad' says https or file" contains "$err" "https://"
  check "mirror '$bad' installs nothing" test ! -e "$D/branch-tally"
done

# 13b. A mirror is announced.
sh_run --to "$D"
check "a file:// mirror prints the effective source" contains "$out" "file://$R"

# 13c. Over https, curl is held to HTTPS and TLS 1.2+.
fresh curlflags
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_curl
run_install BRANCH_TALLY_INSTALL_BASE_URL=https://mirror.example.test "$shell_under_test" "$installer" --to "$D"
check "https mirror install exits 0" test "$rc" -eq 0
check "https mirror prints the effective source" contains "$out" "https://mirror.example.test"
check "curl gets --proto" contains "$S/curl.log" "--proto"
check "curl is restricted to https" contains "$S/curl.log" "=https"
check "curl gets --tlsv1.2" contains "$S/curl.log" "--tlsv1.2"
check "curl restricts redirects to https" contains "$S/curl.log" "--proto-redir"

# 13d. wget (no curl) is held to HTTPS where it can be.
fresh wgetflags
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_wget
TOOLS=$TOOLBIN_NOCURL run_install BRANCH_TALLY_INSTALL_BASE_URL=https://mirror.example.test "$shell_under_test" "$installer" --to "$D"
check "wget https mirror install exits 0" test "$rc" -eq 0
check "wget gets --https-only" contains "$S/wget.log" "--https-only"

# 14. F-06: a tag read from the remote SHA256SUMS must be vMAJOR.MINOR.PATCH.
fresh badtag
for badname in "branch-tally-vevil-$X.tar.gz" "branch-tally-v1.2-$X.tar.gz" "branch-tally-v1.2.3.4-$X.tar.gz" "branch-tally-v../../x-$X.tar.gz"; do
  rm -rf "$R"
  mkdir -p "$R/latest/download"
  printf '%s  %s\n' 0000000000000000000000000000000000000000000000000000000000000000 "$badname" > "$R/latest/download/SHA256SUMS"
  sh_run --to "$D"
  check "remote tag in '$badname' is rejected" test "$rc" -ne 0
  check "remote tag in '$badname' is named in the error" contains "$err" "not of the form vMAJOR.MINOR.PATCH"
  check "remote tag in '$badname' installs nothing" test ! -e "$D/branch-tally"
done

# 15. F-06: a truncated download must not run partway. Cut the script just before its end.
fresh truncated
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
size=$(wc -c < "$installer")
head -c $((size - 150)) "$installer" > "$S/truncated.sh"
run_install "$shell_under_test" "$S/truncated.sh" --to "$D"
check "a truncated script exits non-zero" test "$rc" -ne 0
check "a truncated script installs nothing" test ! -e "$D/branch-tally"
check "a truncated script prints nothing of the install" lacks "$out" "Installing"

# 16. Help documents the new options.
fresh helpnew
sh_run --help
check "--help documents --require-attestation" contains "$out" "--require-attestation"
check "--help documents the env variable" contains "$out" "BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION"
check "--help calls the checksum an integrity check" contains "$out" "integrity"
check "--help mentions gh attestation verify" contains "$out" "gh attestation verify"
check "--help mentions cosign" contains "$out" "cosign"
check "--help limits mirrors to https and file" contains "$out" "file://"

# 17. F-04: "latest" comes from the GitHub API, not from the mirror.
API_TAG_URL=https://api.github.com/repos/IAmSkweetis/branch-tally/releases/latest
fresh api_wins
mk_release "$R" v1.0.0 $X 1.0.0
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v1.0.0   # the mirror's latest/ is stale (a rollback)
mk_api v2.0.0           # GitHub says v2.0.0
sh_run --to "$D"
check "api: exits 0" test "$rc" -eq 0
check "api: installs the release GitHub names, not the mirror's latest" contains "$out" "branch-tally 2.0.0"
check "api: no fallback warning when the API answered" lacks "$err" "not confirmed"

# 17b. Fallback: API unreachable, rate limited or garbage. Warn loudly and use the mirror's latest.
fresh api_down
mk_release "$R" v1.0.0 $X 1.0.0
mk_latest "$R" v1.0.0
rm -rf "$S/api"
sh_run --to "$D"
check "api down: falls back and exits 0" test "$rc" -eq 0
check "api down: installs the mirror's latest" contains "$out" "branch-tally 1.0.0"
check "api down: warns latest was not confirmed by GitHub" contains "$err" "not confirmed by GitHub"
check "api down: says an older release could be served" contains "$err" "older release"
check "api down: names the mirror as the source" contains "$err" "mirror"
check "api down: prints the --version route" contains "$err" "--version"
rm -rf "$D"
mk_gh 0 0
sh_run --require-attestation --to "$D"
check "api down: allowed under --require-attestation" test "$rc" -eq 0
check "api down: still warns under --require-attestation" contains "$err" "not confirmed by GitHub"

fresh api_garbage
mk_release "$R" v1.0.0 $X 1.0.0
mk_latest "$R" v1.0.0
for garbage in '<html>rate limited</html>' '{"message":"API rate limit exceeded"}' '' '{"tag_name":"v1.2"}' '{"tag_name":"evil"}' \
  '{"tag_name":"v1.0.0;rm -rf /"}' '{"tag_name":"v1.0.0.1"}' '{"tag_name":"v../../x"}'; do
  set_api "$garbage"
  sh_run --to "$D"
  check "api garbage '$garbage': falls back with a warning" contains "$err" "not confirmed by GitHub"
  check "api garbage '$garbage': installs the mirror's latest" contains "$out" "branch-tally 1.0.0"
done

# 17c. A minified response is parsed too, and a tag_name inside another field is not taken.
fresh api_min
mk_release "$R" v1.0.0 $X 1.0.0
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v1.0.0
set_api '{"url":"x","body":"see \"tag_name\": \"v1.0.0\" here","tag_name":"v2.0.0","assets":[]}'
sh_run --to "$D"
check "api minified: reads the tag" contains "$out" "branch-tally 2.0.0"
check "api minified: no warning" lacks "$err" "not confirmed"

# 17d. The API base override: https:// or file:// only, printed when set.
fresh api_scheme
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
for bad in http://api.example.test ftp://api.example.test api.example.test; do
  run_install BRANCH_TALLY_INSTALL_API_URL="$bad" "$shell_under_test" "$installer" --to "$D"
  check "api base '$bad' is rejected" test "$rc" -ne 0
  check "api base '$bad' names the variable" contains "$err" "BRANCH_TALLY_INSTALL_API_URL"
  check "api base '$bad' installs nothing" test ! -e "$D/branch-tally"
done
sh_run --to "$D"
check "a custom api base is printed" contains "$out" "Using release API: file://$S/api"
run_install BRANCH_TALLY_INSTALL_API_URL= "$shell_under_test" "$installer" --version v2.0.0 --to "$D"
check "an empty api base means the default (nothing printed)" lacks "$out" "Using release API"

# 17e. Pinned version: the API is not consulted.
fresh api_pinned
mk_release "$R" v1.0.0 $X 1.0.0
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
rm -rf "$S/api"
sh_run --version v1.0.0 --to "$D"
check "pinned: exits 0 without the API" test "$rc" -eq 0
check "pinned: no fallback warning" lacks "$err" "not confirmed"

# 17f. Over https: the default API URL, and the token as a bearer header that is never printed.
fresh api_https
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_curl
run_install BRANCH_TALLY_INSTALL_BASE_URL=https://mirror.example.test BRANCH_TALLY_INSTALL_API_URL= \
  "$shell_under_test" "$installer" --to "$D"
check "default api: exits 0" test "$rc" -eq 0
check "default api: asks api.github.com releases/latest" contains "$S/curl.urls" "$API_TAG_URL"
check "default api: no Authorization without a token" lacks "$S/curl.hdr" "Authorization"
check "default api: curl is held to https" contains "$S/curl.log" "=https"

fresh api_token
mk_release "$R" v2.0.0 $X 2.0.0
mk_latest "$R" v2.0.0
mk_curl
run_install BRANCH_TALLY_INSTALL_BASE_URL=https://mirror.example.test BRANCH_TALLY_INSTALL_API_URL=https://api.example.test \
  GH_TOKEN=ghtok-AAA GITHUB_TOKEN=ghtok-BBB "$shell_under_test" "$installer" --to "$D"
check "token: exits 0" test "$rc" -eq 0
check "token: prints the custom API" contains "$out" "Using release API: https://api.example.test"
check "token: GH_TOKEN is sent as a bearer token" contains "$S/curl.hdr" "Authorization: Bearer ghtok-AAA"
check "token: GH_TOKEN wins over GITHUB_TOKEN" lacks "$S/curl.hdr" "ghtok-BBB"
check "token: never on the command line" lacks "$S/curl.log" "ghtok-"
check "token: never on stdout" lacks "$out" "ghtok-"
check "token: never on stderr" lacks "$err" "ghtok-"
: > "$S/curl.hdr"
run_install BRANCH_TALLY_INSTALL_BASE_URL=https://mirror.example.test BRANCH_TALLY_INSTALL_API_URL=https://api.example.test \
  GITHUB_TOKEN=ghtok-BBB "$shell_under_test" "$installer" --to "$D"
check "token: GITHUB_TOKEN is used when GH_TOKEN is unset" contains "$S/curl.hdr" "Authorization: Bearer ghtok-BBB"
check "token: GITHUB_TOKEN never on stderr" lacks "$err" "ghtok-"
check "token: no token is sent to the mirror" test "$(grep -c Authorization "$S/curl.hdr")" -eq 1

# 17g. A failing API call with a token still never prints it.
fresh api_token_fail
mk_release "$R" v1.0.0 $X 1.0.0
mk_latest "$R" v1.0.0
rm -rf "$S/api"
GH_TOKEN=ghtok-CCC sh_run --to "$D"
check "token on failure: fallback warns" contains "$err" "not confirmed by GitHub"
check "token on failure: never printed" lacks "$err" "ghtok-CCC"

# 17h. A trailing newline is not a valid version (case patterns reject it, unlike a regexp's $).
fresh nl_version
mk_release "$R" v1.0.0 $X 1.0.0
mk_latest "$R" v1.0.0
NL='
'
sh_run --version "v1.0.0$NL" --to "$D"
check "version with a trailing newline is rejected" test "$rc" -ne 0
check "version with a trailing newline installs nothing" test ! -e "$D/branch-tally"

# 17i. --help documents the API base and the fallback.
sh_run --help
check "--help documents the API base variable" contains "$out" "BRANCH_TALLY_INSTALL_API_URL"
check "--help mentions GH_TOKEN" contains "$out" "GH_TOKEN"
check "--help explains the mirror fallback" contains "$out" "not confirmed by GitHub"

printf '\n%s cases run, %s failed\n' "$cases" "$failures"
[ "$failures" -eq 0 ] && [ "$cases" -gt 0 ]
