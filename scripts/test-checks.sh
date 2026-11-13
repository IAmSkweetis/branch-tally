#!/bin/sh
# Self-test for the commit-subject and branch-name checks. A gate that never fires is worse than no
# gate, so this proves both directions: every "bad" case must be rejected and every "good" case
# accepted. Prints the case count and fails if any case went the wrong way.
set -u

dir=$(dirname "$0")
total=0
wrong=0

# expect <accept|reject> <script> [args...]
expect() {
  want=$1
  shift
  total=$((total + 1))
  sh "$@" >/dev/null 2>&1
  got=$?
  if { [ "$want" = accept ] && [ "$got" -ne 0 ]; } || { [ "$want" = reject ] && [ "$got" -eq 0 ]; }; then
    wrong=$((wrong + 1))
    echo "WRONG: expected $want, exit $got: $*" >&2
  fi
}

commit="$dir/check-commit-subject.sh"
for s in "feat: add x" "fix(core): y" "feat(core)!: break" "ci: hooks" "chore(agents): z" \
  "docs: a" "style: a" "refactor: a" "perf: a" "test: a" "build(deps): bump x" \
  "Merge branch 'main' into feat/x"; do
  expect accept "$commit" "$s"
done
for s in "added stuff" "Feat: x" "feat:x" "feat(Core): x" "feat(): x" "feat : x" "feature: x" \
  "wip" "" "feat: " "fix(a b): x"; do
  expect reject "$commit" "$s"
done

branch="$dir/check-branch-name.sh"
for b in main feat/issue-4-report fix/x chore/issue-17-gates-tooling feature/a bugfix/a \
  hotfix/a-b release/v0.3.0 release/1.0.0-rc1; do
  expect accept "$branch" "$b"
done
for b in scott-work feat/Bad feat/a--b feat/-a feat/a- fix/a_b feat/ feat "feat/a b" \
  dependabot/cargo/serde-1.2 wip/thing "" release/v0.3.0_x; do
  expect reject "$branch" "$b"
done
expect accept "$branch" --allow-dependabot dependabot/github_actions/actions/checkout-7
expect accept "$branch" --allow-dependabot chore/ok
expect reject "$branch" --allow-dependabot scott-work

# The packaging script: bad versions and targets are refused, a good one yields an archive holding
# the binary, LICENSE and README.md. A fake binary stands in, so this needs no build.
package="$dir/package.sh"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
printf 'fake\n' >"$scratch/branch-tally"
export DIST_DIR="$scratch/out"
linux=x86_64-unknown-linux-musl
for v in v0.1.0 v10.20.30; do
  expect accept "$package" "$v" "$linux" "$scratch/branch-tally"
done
for v in 0.1.0 v1.2 v1.2.3-rc1 vx.y.z ""; do
  expect reject "$package" "$v" "$linux" "$scratch/branch-tally"
done
expect reject "$package" v0.1.0 "" "$scratch/branch-tally"
expect reject "$package" v0.1.0 "$linux" "$scratch/missing"
total=$((total + 1))
name="branch-tally-v0.1.0-$linux"
listing=$(tar -tzf "$scratch/out/$name.tar.gz" 2>/dev/null | LC_ALL=C sort | tr '\n' ' ')
want="$name/ $name/LICENSE $name/README.md $name/branch-tally "
if [ "$listing" != "$want" ]; then
  wrong=$((wrong + 1))
  echo "WRONG: archive contents were: $listing" >&2
fi

# The installer gates (lint-install.sh, test-install.sh). Each runs in a scratch tree with a PATH
# holding only the tools a case names, so what is on this machine cannot change the verdict. A
# missing tool is a skip with a message locally and a failure when CI is set; a failing tool fails
# either way. `inst <case-dir> <script> <accept|reject>` runs one case; `out` holds its output.
posix_sh=$(command -v sh)
mkinst() {
  # mkinst <name>: a tree with the scripts, the four installer files and a PATH dir with dirname.
  d="$scratch/$1"
  mkdir -p "$d/scripts" "$d/tests/install" "$d/bin"
  cp "$dir/lint-install.sh" "$dir/test-install.sh" "$d/scripts/" 2>/dev/null
  : >"$d/install.sh"
  : >"$d/install.ps1"
  : >"$d/tests/install/install_test.sh"
  : >"$d/tests/install/install_test.ps1"
  # Wrappers, not symlinks: Git Bash without symlink privilege turns `ln -s` into a copy, and a
  # copied MSYS binary can't load msys-2.0.dll once PATH holds only this dir. Exec'ing the real
  # binary by absolute path keeps its DLLs beside it on every platform.
  wrap "$d/bin/dirname" "$(command -v dirname)"
  wrap "$d/bin/sh" "$posix_sh"
}
wrap() {
  # wrap <path> <real-binary>: a stand-in that runs the real binary with the same arguments.
  printf '#!%s\nexec "%s" "$@"\n' "$posix_sh" "$2" >"$1"
  chmod +x "$1"
}
fake() {
  # fake <case> <tool> <exit>: a stand-in tool that exits with the given status.
  printf '#!/bin/sh\nexit %s\n' "$3" >"$scratch/$1/bin/$2"
  chmod +x "$scratch/$1/bin/$2"
}
inst() {
  # inst <case> <script> <accept|reject> [env assignment]
  want=$3
  total=$((total + 1))
  out=$(cd "$scratch/$1" && env -i PATH="$scratch/$1/bin" ${4:+"$4"} "$posix_sh" "scripts/$2" 2>&1)
  got=$?
  if { [ "$want" = accept ] && [ "$got" -ne 0 ]; } || { [ "$want" = reject ] && [ "$got" -eq 0 ]; }; then
    wrong=$((wrong + 1))
    echo "WRONG: $1: $2 expected $want, exit $got: $out" >&2
  fi
}
skipped() {
  # skipped <case>: the last inst run must have said it skipped, never passed silently.
  total=$((total + 1))
  case $out in *skipped*) ;; *) wrong=$((wrong + 1)); echo "WRONG: $1: no skip message: $out" >&2 ;; esac
}

for script in lint-install.sh test-install.sh; do
  # No tools: skipped (loudly) locally, an error in CI.
  mkinst "$script-bare"
  inst "$script-bare" "$script" accept
  skipped "$script-bare"
  inst "$script-bare" "$script" reject CI=true
  # A missing installer file is an error, never a skip.
  mkinst "$script-nofile"
  fake "$script-nofile" shellcheck 0
  rm "$scratch/$script-nofile/install.sh"
  inst "$script-nofile" "$script" reject
done

# lint-install.sh: a tool that finds problems fails the gate even when the other tool is absent.
mkinst lint-bad
fake lint-bad shellcheck 1
inst lint-bad lint-install.sh reject
mkinst lint-ok
fake lint-ok shellcheck 0
inst lint-ok lint-install.sh accept
skipped lint-ok # pwsh is absent: that half is reported as skipped
mkinst lint-psbad
fake lint-psbad shellcheck 0
fake lint-psbad pwsh 1
inst lint-psbad lint-install.sh reject
fake lint-psbad pwsh 0
inst lint-psbad lint-install.sh accept

# test-install.sh: a failing sh test fails the gate, a failing pwsh test too, two passes pass.
mkinst test-shfail
printf 'exit 1\n' >"$scratch/test-shfail/tests/install/install_test.sh"
fake test-shfail pwsh 0
inst test-shfail test-install.sh reject
mkinst test-psfail
fake test-psfail pwsh 1
inst test-psfail test-install.sh reject
fake test-psfail pwsh 0
inst test-psfail test-install.sh accept

# On a Windows host install.sh is not the installer (install.ps1 is), so the sh suite is skipped
# with a message while the pwsh suite still runs and still fails the gate. A Windows run without
# pwsh stays a failure in CI: the skip must not become a false green. The sh test here always
# exits 1, so an accept means it was skipped, and the pwsh stand-in's status decides the rest.
fakeuname() {
  # fakeuname <case> <kernel name>: a stand-in uname that prints the name.
  rm -f "$scratch/$1/bin/uname"
  printf '#!/bin/sh\necho %s\n' "$2" >"$scratch/$1/bin/uname"
  chmod +x "$scratch/$1/bin/uname"
}
said() {
  # said <case> <text>: the last inst run's output must contain the text.
  total=$((total + 1))
  case $out in *"$2"*) ;; *) wrong=$((wrong + 1)); echo "WRONG: $1: output lacks '$2': $out" >&2 ;; esac
}
for kernel in MINGW64_NT-10.0-26100 MSYS_NT-10.0 CYGWIN_NT-10.0; do
  c="win-$kernel"
  mkinst "$c"
  printf 'exit 1\n' >"$scratch/$c/tests/install/install_test.sh"
  fakeuname "$c" "$kernel"
  fake "$c" pwsh 0
  inst "$c" test-install.sh accept
  said "$c" "install.sh does not target Windows; install_test.ps1 covers it"
  fake "$c" pwsh 1
  inst "$c" test-install.sh reject
  rm "$scratch/$c/bin/pwsh"
  inst "$c" test-install.sh reject CI=true
  inst "$c" test-install.sh accept
  skipped "$c"
done
# The OS variable alone marks a Windows host, with no uname on the PATH at all.
mkinst win-env
printf 'exit 1\n' >"$scratch/win-env/tests/install/install_test.sh"
fake win-env pwsh 0
inst win-env test-install.sh accept OS=Windows_NT
said win-env "install.sh does not target Windows"
# A Linux host still runs the sh suite: its failure fails the gate.
mkinst linux-shfail
printf 'exit 1\n' >"$scratch/linux-shfail/tests/install/install_test.sh"
fakeuname linux-shfail Linux
fake linux-shfail pwsh 0
inst linux-shfail test-install.sh reject

# ci-detect.sh: only the exact string "false" (a docs-only PR, from the `changes` job) skips the
# Rust checks. Empty (a push, or `changes` failed) and "true" must run them, or a failed filter
# would pass a PR unchecked.
detect_sh=$(cd "$dir" && pwd)/ci-detect.sh
mkdir "$scratch/detect"
printf '[workspace.package]\nrust-version = "1.99.1"\n' >"$scratch/detect/Cargo.toml"
: >"$scratch/detect/deny.toml"
detect() {
  # detect <CODE_CHANGED value|unset> <expected rust=> : runs in a repo that has a Cargo.toml.
  total=$((total + 1))
  if [ "$1" = unset ]; then
    got=$(cd "$scratch/detect" && env -u CODE_CHANGED GITHUB_OUTPUT=/dev/stdout sh "$detect_sh" 2>&1)
  else
    got=$(cd "$scratch/detect" && CODE_CHANGED=$1 GITHUB_OUTPUT=/dev/stdout sh "$detect_sh" 2>&1)
  fi
  case $got in
    *"rust=$2"*) ;;
    *) wrong=$((wrong + 1)); echo "WRONG: ci-detect CODE_CHANGED=$1: expected rust=$2: $got" >&2 ;;
  esac
}
detect false false
detect true true
detect "" true
detect unset true
detect FALSE true
total=$((total + 1))
case $(cd "$scratch/detect" && CODE_CHANGED=false GITHUB_OUTPUT=/dev/stdout sh "$detect_sh" 2>&1) in
  *"::notice title=Rust checks skipped"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: ci-detect skipped without a notice" >&2 ;;
esac

# ci-tally-check.sh: fetches refs/pull/<N>/merge as data, derives base (first parent) and head (the
# merge commit), skips when .tally/ is untouched, and otherwise runs the binary inside a
# ::stop-commands:: block, translating its `warning: ` and `note: ` lines. Needs a stand-in binary
# and a local "remote" that serves refs/pull/<N>/merge.
tally_sh="$(cd "$dir" && pwd)/ci-tally-check.sh"
tr="$scratch/tally"
mkdir -p "$tr/remote" "$tr/work"
(
  cd "$tr/remote" || exit 1
  git init -q -b main . && git config user.email t@example.com && git config user.name t
  git config commit.gpgsign false
  echo a >a.txt && git add a.txt && git commit -q -m "chore: base"
  # PR 1 touches .tally/. Main moves on after the PR branched, so the merge commit's first parent
  # differs from the PR's recorded base (the stale-base case).
  git checkout -q -b pr1 && mkdir .tally && echo '{}' >.tally/ledger.jsonl &&
    git add .tally && git commit -q -m "chore: tally"
  git checkout -q main && echo m >m.txt && git add m.txt && git commit -q -m "chore: main moves"
  git rev-parse HEAD >"$scratch/tally-base1"
  git checkout -q -b merge1 && git merge -q --no-ff -m merge pr1
  git update-ref refs/pull/1/merge HEAD
  # PR 2 does not touch .tally/.
  git checkout -q main && git checkout -q -b pr2 && echo b >b.txt && git add b.txt &&
    git commit -q -m "chore: no tally"
  git checkout -q main && git checkout -q -b merge2 && git merge -q --no-ff -m merge pr2
  git update-ref refs/pull/2/merge HEAD
  # PR 4's merge ref is stale on the base side: its second parent is the PR head, but its first
  # parent is main before it moved, so it no longer matches the base branch tip.
  git checkout -q -b merge4 main~1 && git merge -q --no-ff -m merge pr1
  git update-ref refs/pull/4/merge HEAD
  git checkout -q main
  cd ../work && git init -q -b main . && git remote add origin "$tr/remote" &&
    git fetch -q origin main && git reset -q --hard FETCH_HEAD
) >/dev/null 2>&1
# tally_bin <name> <exit> <output lines...>: a stand-in binary that records its arguments.
tally_bin() {
  name=$1 code=$2
  shift 2
  {
    printf '#!/bin/sh\nprintf "%%s\\n" "$*" >"%s.args"\n' "$scratch/$name"
    for line in "$@"; do printf 'printf "%%s\\n" %s >&2\n' "'$line'"; done
    printf 'exit %s\n' "$code"
  } >"$scratch/$name"
  chmod +x "$scratch/$name"
}
tally_run() {
  # tally_run <binary> <pr>: the script's combined output; its exit status is the function's.
  # PR_HEAD_SHA defaults to the merge ref's second parent (a fresh ref); a test overrides it.
  sha=${PR_HEAD_SHA-$(git -C "$tr/remote" rev-parse --verify --quiet "refs/pull/${2:-0}/merge^2" 2>/dev/null || echo)}
  (cd "$tr/work" && HEAD_REF=feat/x BASE_REF=main PR_HEAD_SHA=$sha \
    TALLY_CHECK_RETRY_DELAY=${TALLY_CHECK_RETRY_DELAY-0} sh "$tally_sh" "$@" 2>&1)
}
tally_bin tb-ok 0 "note: 2 new ledger line(s) recorded for branch other" \
  "warning: .tally/prices.toml differs from base"
tally_bin tb-viol 1 "error: bad line"
tally_bin tb-inject 0 "::error::pwned" "warning: ::error::nested" "::add-mask::secret"
tally_bin tb-bslash 0 'note: 1 new ledger line(s) recorded for branch x\n::error::forged'

# a passing check, with notice and warning translation, and the exact interface
total=$((total + 1))
out=$(tally_run "$scratch/tb-ok" 1)
rc=$?
[ "$rc" -eq 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check passing run exited $rc: $out" >&2; }
case $out in
  *"::notice::2 new ledger line(s) recorded for branch other"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: tally-check did not translate note: to ::notice::: $out" >&2 ;;
esac
case $out in
  *"::warning::.tally/prices.toml differs from base"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: tally-check did not translate warning: to ::warning::: $out" >&2 ;;
esac
# base = first parent of the merge (main after it moved), head = the merge commit
want_base=$(cat "$scratch/tally-base1")
want_head=$(git -C "$tr/remote" rev-parse refs/pull/1/merge)
total=$((total + 1))
case $(cat "$scratch/tb-ok.args") in
  "ledger check --base $want_base --head $want_head --head-branch feat/x --base-branch main") ;;
  *) wrong=$((wrong + 1)); echo "WRONG: tally-check passed the wrong arguments: $(cat "$scratch/tb-ok.args")" >&2 ;;
esac

# a failing check fails the job
total=$((total + 1))
tally_run "$scratch/tb-viol" 1 >/dev/null && { wrong=$((wrong + 1)); echo "WRONG: tally-check passed a failing binary" >&2; }

# nothing changed under .tally/: skipped quickly, reports success, never runs the binary
rm -f "$scratch/tb-viol.args"
total=$((total + 1))
out=$(tally_run "$scratch/tb-viol" 2)
rc=$?
if [ "$rc" -ne 0 ] || [ -e "$scratch/tb-viol.args" ]; then
  wrong=$((wrong + 1)); echo "WRONG: tally-check did not skip an untouched .tally/ (exit $rc): $out" >&2
fi
case $out in
  *"::notice"*"no .tally/"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: tally-check skip printed no notice: $out" >&2 ;;
esac

# a missing merge ref (conflicted PR) fails, never passes
total=$((total + 1))
out=$(PR_HEAD_SHA=$(git -C "$tr/remote" rev-parse refs/pull/1/merge^2) tally_run "$scratch/tb-ok" 99)
rc=$?
[ "$rc" -ne 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check passed a missing merge ref" >&2; }
case $out in
  *"::error"*"merge"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: missing merge ref gave no clear error: $out" >&2 ;;
esac

# head-SHA check: the merge ref must be built from the head the event named. The test delay is 0
# (or 2s where a background update must land between attempts).
sha1=$(git -C "$tr/remote" rev-parse refs/pull/1/merge^2)
merge1=$(git -C "$tr/remote" rev-parse refs/pull/1/merge)
merge2=$(git -C "$tr/remote" rev-parse refs/pull/2/merge)
git -C "$tr/remote" update-ref refs/pull/3/merge "$merge2"
git -C "$tr/remote" update-ref refs/pull/5/merge "$merge2"

# stale merge ref that never catches up: fails after the retries, says so, never runs the binary
rm -f "$scratch/tb-ok.args"
total=$((total + 1))
out=$(PR_HEAD_SHA=$sha1 tally_run "$scratch/tb-ok" 3)
rc=$?
[ "$rc" -ne 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check passed a stale merge ref" >&2; }
[ ! -e "$scratch/tb-ok.args" ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check ran the binary on a stale merge ref" >&2; }
case $out in
  *"::error"*"stale"*"Re-run"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: stale merge ref gave no clear error: $out" >&2 ;;
esac

# a stale ref that catches up on a retry passes, and reads the caught-up merge commit
rm -f "$scratch/tb-ok.args"
total=$((total + 1))
(sleep 1; git -C "$tr/remote" update-ref refs/pull/5/merge "$merge1") &
updater=$!
out=$(PR_HEAD_SHA=$sha1 TALLY_CHECK_RETRY_DELAY=2 tally_run "$scratch/tb-ok" 5)
rc=$?
wait "$updater"
[ "$rc" -eq 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check did not pass a ref that caught up (exit $rc): $out" >&2; }
case $(cat "$scratch/tb-ok.args" 2>/dev/null) in
  *"--head $merge1 "*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: tally-check did not read the caught-up merge commit" >&2 ;;
esac

# a missing, empty or malformed PR_HEAD_SHA fails
nothex=$(printf '%040d' 0 | tr 0 g)
for bad in "" abc "${sha1}0" "$nothex"; do
  total=$((total + 1))
  out=$(PR_HEAD_SHA=$bad tally_run "$scratch/tb-ok" 1)
  rc=$?
  [ "$rc" -ne 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check accepted PR_HEAD_SHA='$bad'" >&2; }
  case $out in
    *"::error"*PR_HEAD_SHA*) ;;
    *) wrong=$((wrong + 1)); echo "WRONG: bad PR_HEAD_SHA='$bad' gave no clear error: $out" >&2 ;;
  esac
done
total=$((total + 1))
(cd "$tr/work" && env -u PR_HEAD_SHA HEAD_REF=feat/x BASE_REF=main sh "$tally_sh" "$scratch/tb-ok" 1 >/dev/null 2>&1) &&
  { wrong=$((wrong + 1)); echo "WRONG: tally-check ran with PR_HEAD_SHA unset" >&2; }

# a fetch failure (unreachable remote) and a missing ref give distinct messages, both fail
total=$((total + 1))
out=$(cd "$tr/work" && REMOTE="$scratch/no-such-remote" HEAD_REF=feat/x BASE_REF=main PR_HEAD_SHA=$sha1 \
  sh "$tally_sh" "$scratch/tb-ok" 1 2>&1)
rc=$?
[ "$rc" -ne 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check passed an unreachable remote" >&2; }
case $out in
  *"::error"*"could not fetch"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: fetch failure gave no fetch-failure error: $out" >&2 ;;
esac
case $out in
  *"does not exist"*) wrong=$((wrong + 1)); echo "WRONG: fetch failure was reported as a missing ref: $out" >&2 ;;
esac
total=$((total + 1))
out=$(PR_HEAD_SHA=$sha1 tally_run "$scratch/tb-ok" 99)
case $out in
  *"::error"*"does not exist"*"conflict"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: missing ref did not say it does not exist: $out" >&2 ;;
esac
case $out in
  *"could not fetch"*) wrong=$((wrong + 1)); echo "WRONG: missing ref was reported as a fetch failure: $out" >&2 ;;
esac

# an empty stop-commands token fails closed (a stub od stands in for a broken generator)
mkdir -p "$scratch/noop-od"
printf '#!/bin/sh\nexit 0\n' >"$scratch/noop-od/od"
chmod +x "$scratch/noop-od/od"
total=$((total + 1))
out=$(PATH="$scratch/noop-od:$PATH" tally_run "$scratch/tb-ok" 1)
rc=$?
[ "$rc" -ne 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check passed with an empty stop-commands token" >&2; }
case $out in
  *"::error"*token*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: empty token gave no clear error: $out" >&2 ;;
esac

# a missing binary, missing refs and a non-numeric PR number all fail
total=$((total + 1))
tally_run "$scratch/tb-missing" 1 >/dev/null && { wrong=$((wrong + 1)); echo "WRONG: tally-check passed a missing binary" >&2; }
total=$((total + 1))
(cd "$tr/work" && sh "$tally_sh" "$scratch/tb-ok" 1 >/dev/null 2>&1) &&
  { wrong=$((wrong + 1)); echo "WRONG: tally-check ran without HEAD_REF/BASE_REF" >&2; }
total=$((total + 1))
tally_run "$scratch/tb-ok" "1;echo" >/dev/null && { wrong=$((wrong + 1)); echo "WRONG: tally-check accepted a non-numeric PR number" >&2; }

# workflow-command injection: the binary's output sits inside a stop-commands block whose token it
# cannot know, and nothing outside the block is a command from it
total=$((total + 1))
out=$(tally_run "$scratch/tb-inject" 1)
rc=$?
[ "$rc" -eq 0 ] || { wrong=$((wrong + 1)); echo "WRONG: injection run exited $rc: $out" >&2; }
verdict=$(printf '%s\n' "$out" | awk '
  /^::stop-commands::/ && !open && !done { tok = substr($0, 18); open = 1; next }
  open && $0 == "::" tok "::" { open = 0; done = 1; next }
  open { next }
  /^::error::pwned|^::add-mask::/ { bad = 1 }
  END { print (done && !bad) ? "ok" : "bad" }')
[ "$verdict" = ok ] || { wrong=$((wrong + 1)); echo "WRONG: injected command escaped the stop-commands block: $out" >&2; }
# the re-emitted warning carries the injected text as data, on one ::warning:: line
case $out in
  *"::warning::::error::nested"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: injected warning was not re-emitted as plain warning text: $out" >&2 ;;
esac

# a merge ref whose first parent is not the current base tip is stale too: it fails after the
# retries and never runs the binary, so an outdated base can't hide the PR's .tally/ changes
total=$((total + 1))
rm -f "$scratch/tb-ok.args"
out=$(tally_run "$scratch/tb-ok" 4)
rc=$?
[ "$rc" -ne 0 ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check passed a merge ref built on an old base" >&2; }
[ ! -e "$scratch/tb-ok.args" ] || { wrong=$((wrong + 1)); echo "WRONG: tally-check ran the binary on a merge ref built on an old base" >&2; }
case $out in
  *"::error"*"stale"*"base"*) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: an old-base merge ref gave no clear error: $out" >&2 ;;
esac

# a ledger branch name holding a literal backslash-n stays one annotation line: dash's echo would
# expand it into a second line that starts a workflow command outside the stop-commands block
total=$((total + 1))
out=$(tally_run "$scratch/tb-bslash" 1)
case $(printf '%s\n' "$out" | grep -c '^::error::forged') in
  0) ;;
  *) wrong=$((wrong + 1)); echo "WRONG: a backslash-n in a note escaped as a workflow command: $out" >&2 ;;
esac

echo "check scripts: $total cases, $wrong wrong"
[ "$wrong" -eq 0 ]
