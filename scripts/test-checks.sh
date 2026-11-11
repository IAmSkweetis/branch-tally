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
  ln -s "$(command -v dirname)" "$d/bin/dirname"
  ln -s "$posix_sh" "$d/bin/sh"
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

echo "check scripts: $total cases, $wrong wrong"
[ "$wrong" -eq 0 ]
