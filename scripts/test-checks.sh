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
listing=$(tar -tzf "$scratch/out/$name.tar.gz" 2>/dev/null | sort | tr '\n' ' ')
want="$name/ $name/LICENSE $name/README.md $name/branch-tally "
if [ "$listing" != "$want" ]; then
  wrong=$((wrong + 1))
  echo "WRONG: archive contents were: $listing" >&2
fi

echo "check scripts: $total cases, $wrong wrong"
[ "$wrong" -eq 0 ]
