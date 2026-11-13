#!/bin/sh
# CI: the `tally-check` job's logic (.github/workflows/pr-checks.yml).
#
#   ci-tally-check.sh changed <base-sha>
#       Writes changed=true|false to $GITHUB_OUTPUT (stdout when unset): did the PR touch .tally/?
#       Fails on a bad base ref, so an unreachable base is never read as "nothing changed".
#   ci-tally-check.sh run <binary> <base-sha> <head-branch>
#       Runs `ledger check` with a binary built from the BASE commit, so a PR that edits the
#       checker cannot disable its own check. If that binary predates the subcommand (the PR that
#       introduces it), the check is skipped with a ::warning:: and passes. It never falls back to
#       building the PR's own code. A missing binary or a failing check fails.
set -eu

mode=${1:-}
case $mode in
  changed)
    base=${2:?usage: ci-tally-check.sh changed <base-sha>}
    out="${GITHUB_OUTPUT:-/dev/stdout}"
    git rev-parse --verify --quiet "$base^{commit}" >/dev/null ||
      { echo "::error::base commit $base is not in this checkout (fetch more history)" >&2; exit 1; }
    files=$(git diff --name-only "$base...HEAD" -- .tally/)
    if [ -n "$files" ]; then
      echo "changed=true" >>"$out"
    else
      echo "changed=false" >>"$out"
      echo "::notice title=tally-check::no .tally/ file changed in this pull request"
    fi
    ;;
  run)
    bin=${2:?usage: ci-tally-check.sh run <binary> <base-sha> <head-branch>}
    base=${3:?usage: ci-tally-check.sh run <binary> <base-sha> <head-branch>}
    head=${4:?usage: ci-tally-check.sh run <binary> <base-sha> <head-branch>}
    if [ ! -x "$bin" ]; then
      echo "::error::base-built binary $bin not found; the base build did not produce it" >&2
      exit 1
    fi
    if ! "$bin" ledger check --help >/dev/null 2>&1; then
      echo "::warning title=tally-check skipped::the base commit predates 'ledger check', so the .tally/ check was skipped"
      exit 0
    fi
    "$bin" ledger check --base "$base" --head-branch "$head"
    ;;
  *)
    echo "usage: ci-tally-check.sh changed <base-sha> | run <binary> <base-sha> <head-branch>" >&2
    exit 2
    ;;
esac
