#!/bin/sh
# Smoke-test a built release binary on the machine that built it:
#
#   smoke-test.sh <path/to/binary> <X.Y.Z>
#
# 1. `--version` must print exactly `branch-tally X.Y.Z` (no leading v).
# 2. `report` must exit 0 and print a non-empty JSON claim against the committed Claude Code
#    fixture `main-then-branch`, in a throwaway git repository with a throwaway HOME, so it never
#    reads the runner's real transcripts.
# Run from the repository root. Used by build.yml, and runnable locally.
set -eu

[ $# -eq 2 ] || { echo "usage: smoke-test.sh <binary> <X.Y.Z>" >&2; exit 2; }
bin=$1
version=$2
[ -f "$bin" ] || { echo "error: binary not found: $bin" >&2; exit 1; }
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")

got=$("$bin" --version | tr -d '\r')
want="branch-tally $version"
if [ "$got" != "$want" ]; then
  echo "error: --version printed '$got', expected '$want'" >&2
  exit 1
fi
echo "--version: $got"

fixture="$(pwd)/tests/fixtures/claude-code/main-then-branch"
[ -d "$fixture/C--repo" ] || { echo "error: fixture not found: $fixture (run from the repo root)" >&2; exit 1; }

# Inside the checkout's target/ (gitignored), not the system temp dir: on Windows runners the temp
# path is an 8.3 short name, which would not match the slug the binary derives from the real path.
mkdir -p target
work=$(mktemp -d "$(pwd)/target/smoke.XXXXXX")
trap 'rm -rf "$work"' EXIT
repo="$work/repo"
home="$work/home"
mkdir -p "$repo" "$home/.claude/projects"
# Native form of the repository path: `pwd -W` on Git Bash, else the plain path.
native=$(cd "$repo" && { pwd -W 2>/dev/null || pwd -P; })
slug=$(printf '%s' "$native" | sed 's/[^A-Za-z0-9]/-/g')
cp -R "$fixture/C--repo" "$home/.claude/projects/$slug"

git_quiet() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git \
    -c user.name=bt -c user.email=bt@example.invalid -c commit.gpgsign=false -C "$repo" "$@"
}
git_quiet init -q -b main
git_quiet commit -q --allow-empty -m init

# `feat/new` is the branch the fixture's records name.
out=$(cd "$repo" && HOME="$home" USERPROFILE="$home" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
  "$bin" report feat/new --json)
case $out in
  *'"totalCalls": 5'* | *'"totalCalls":5'*) echo "report: ok (5 calls attributed to feat/new)" ;;
  *) echo "error: report did not attribute the fixture's 5 calls to feat/new" >&2; exit 1 ;;
esac
