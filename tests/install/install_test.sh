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
shell_under_test=${TEST_SHELL:-sh}

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
  tar -C "$stage" -czf "$root/download/$tag/$name.tar.gz" "$name"
  (cd "$root/download/$tag" && $sha_cmd "$name.tar.gz" >> SHA256SUMS)
  rm -rf "$stage"
}

# mk_latest <root> <tag>: make <tag> the latest release.
mk_latest() {
  rm -rf "$1/latest"
  mkdir -p "$1/latest"
  cp -R "$1/download/$2" "$1/latest/download"
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
  mkdir -p "$S/home" "$S/tmp" "$R"
}

# run_install <env assignments and command...>: runs in the sandbox; sets $rc, $out, $err.
run_install() {
  out=$S/out
  err=$S/err
  env -i PATH="$FAKEBIN:$PATH" HOME="$S/home" TMPDIR="$S/tmp" \
    FAKE_UNAME_S="${FAKE_UNAME_S:-Linux}" FAKE_UNAME_M="${FAKE_UNAME_M:-x86_64}" \
    FAKE_PROC_TRANSLATED="${FAKE_PROC_TRANSLATED:-0}" \
    BRANCH_TALLY_INSTALL_BASE_URL="file://$R" "$@" >"$out" 2>"$err" </dev/null
  rc=$?
}

X=x86_64-unknown-linux-musl
# sh_run <installer args...>
sh_run() { run_install $shell_under_test "$installer" "$@"; }

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
  $shell_under_test "$installer"
check "env-pinned version exits 0" test "$rc" -eq 0
check "env-pinned version installs v1.0.0" contains "$out" "branch-tally 1.0.0"

# 2b. Flags override env.
run_install BRANCH_TALLY_INSTALL_DIR="$S/ignored" BRANCH_TALLY_INSTALL_VERSION=v2.0.0 \
  $shell_under_test "$installer" --version v1.0.0 --to "$D"
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
run_install PATH="$D:$FAKEBIN:$PATH" $shell_under_test "$installer" --to "$D"
check "no PATH hint when dir is on PATH" lacks "$out" "export PATH"
run_install PATH="$D/:$FAKEBIN:$PATH" $shell_under_test "$installer" --to "$D"
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
FAKE_UNAME_S=FreeBSD sh_run --to "$D"
check "unsupported OS exits non-zero" test "$rc" -ne 0
check "unsupported OS names the OS" contains "$err" "FreeBSD"
FAKE_UNAME_M=riscv64 sh_run --to "$D"
check "unsupported arch exits non-zero" test "$rc" -ne 0
check "unsupported arch names the arch" contains "$err" "riscv64"

# 9. Platform mapping: Linux arm64, macOS arm64, macOS Intel under Rosetta, native macOS Intel.
fresh plat
for spec in \
  "Linux aarch64 0 aarch64-unknown-linux-musl" \
  "Darwin arm64 0 aarch64-apple-darwin" \
  "Darwin x86_64 1 aarch64-apple-darwin" \
  "Darwin x86_64 0 x86_64-apple-darwin"; do
  set -- $spec
  rm -rf "$R" "$D"
  mkdir -p "$R"
  mk_release "$R" v2.0.0 "$4" 2.0.0
  mk_latest "$R" v2.0.0
  FAKE_UNAME_S=$1 FAKE_UNAME_M=$2 FAKE_PROC_TRANSLATED=$3 sh_run --to "$D"
  check "platform $1/$2 (translated=$3) selects $4" test "$rc" -eq 0
done

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

printf '\n%s cases run, %s failed\n' "$cases" "$failures"
[ "$failures" -eq 0 ] && [ "$cases" -gt 0 ]
