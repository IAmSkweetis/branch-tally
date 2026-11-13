#!/bin/sh
# CI: the `tally-check` job's logic (.github/workflows/tally-check.yml).
#
#   HEAD_REF=<pr head branch> BASE_REF=<pr base branch> [REMOTE=origin] \
#     ci-tally-check.sh <binary> <pr-number>
#
# Runs under `pull_request_target`, so this script and the binary come from the BASE branch. The
# PR's content is only ever fetched as git data (refs/pull/<N>/merge) and read by the binary with
# `--head <rev>`; nothing from it is checked out, built or executed.
#
#   1. Fetch refs/pull/<N>/merge. Missing (the PR has conflicts) fails with a clear message.
#   2. head = the merge commit, base = its first parent, so a stale pull_request.base.sha cannot
#      cause false failures.
#   3. No change under .tally/ between base and head: report success and stop.
#   4. Otherwise run `branch-tally ledger check`. Its output is PR-controlled text (branch names,
#      ledger fields), so it is printed inside a ::stop-commands:: block with a random token, then
#      `warning: ` lines are re-emitted as ::warning:: and `note: ` lines as ::notice::.
#      The job fails iff the binary exits non-zero.
set -eu

bin=${1:?usage: ci-tally-check.sh <binary> <pr-number>}
pr=${2:?usage: ci-tally-check.sh <binary> <pr-number>}
head_ref=${HEAD_REF:?HEAD_REF (the PR head branch) must be set}
base_ref=${BASE_REF:?BASE_REF (the PR base branch) must be set}
remote=${REMOTE:-origin}

case $pr in
  '' | *[!0-9]*) echo "::error::PR number '$pr' is not numeric" >&2; exit 1 ;;
esac
if [ ! -x "$bin" ]; then
  echo "::error::binary $bin not found; the base build did not produce it" >&2
  exit 1
fi

if ! git fetch --quiet --no-tags "$remote" "+refs/pull/$pr/merge:refs/tally-check/merge" 2>/dev/null; then
  echo "::error title=tally-check::refs/pull/$pr/merge does not exist: the PR probably has merge conflicts with its base. Resolve them so the check can run; it will not pass without a merge commit to read." >&2
  exit 1
fi
head=$(git rev-parse --verify --quiet "refs/tally-check/merge^{commit}") ||
  { echo "::error::refs/pull/$pr/merge is not a commit" >&2; exit 1; }
base=$(git rev-parse --verify --quiet "$head^1^{commit}") ||
  { echo "::error::the merge commit has no first parent" >&2; exit 1; }
git rev-parse --verify --quiet "$head^2^{commit}" >/dev/null ||
  { echo "::error::refs/pull/$pr/merge is not a merge commit; refusing to guess a base" >&2; exit 1; }

changed=$(git diff --name-only "$base" "$head" -- .tally/)
if [ -z "$changed" ]; then
  echo "::notice title=tally-check::no .tally/ file changed in this pull request"
  exit 0
fi

out=$(mktemp)
trap 'rm -f "$out"' EXIT
token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
echo "::stop-commands::$token"
code=0
"$bin" ledger check --base "$base" --head "$head" --head-branch "$head_ref" --base-branch "$base_ref" \
  >"$out" 2>&1 || code=$?
cat "$out"
echo "::$token::"

# Re-emit as annotations. The text after the prefix is data: it stays on one line, so it cannot
# start a second command.
while IFS= read -r line || [ -n "$line" ]; do
  line=$(printf '%s' "$line" | tr -d '\r')
  case $line in
    "warning: "*) echo "::warning::${line#warning: }" ;;
    "note: "*) echo "::notice::${line#note: }" ;;
  esac
done <"$out"

if [ "$code" -ne 0 ]; then
  echo "::error title=tally-check::branch-tally ledger check failed (exit $code); see the output above"
fi
exit "$code"
