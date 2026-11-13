#!/bin/sh
# CI: the `tally-check` job's logic (.github/workflows/tally-check.yml).
#
#   HEAD_REF=<pr head branch> BASE_REF=<pr base branch> PR_HEAD_SHA=<pr head sha> \
#     [REMOTE=origin] [TALLY_CHECK_RETRY_DELAY=5] ci-tally-check.sh <binary> <pr-number>
#
# Runs under `pull_request_target`, so this script and the binary come from the BASE branch. The
# PR's content is only ever fetched as git data (refs/pull/<N>/merge) and read by the binary with
# `--head <rev>`; nothing from it is checked out, built or executed.
#
#   1. Fetch refs/pull/<N>/merge. Missing (the PR has conflicts) and unfetchable (network or auth)
#      fail with distinct messages, told apart by `git ls-remote`.
#   2. head = the merge commit, base = its first parent, so a stale pull_request.base.sha cannot
#      cause false failures. The merge commit's second parent must equal PR_HEAD_SHA (from the
#      event, validated as 40 or 64 hex). GitHub can fire the event before it recomputes the merge
#      ref; a mismatch is re-fetched, 5 attempts in all, TALLY_CHECK_RETRY_DELAY (default 5)
#      seconds apart, then fails with a "stale, re-run" error. It never passes on a mismatch.
#   3. No change under .tally/ between base and head: report success and stop.
#   4. Otherwise run `branch-tally ledger check`. Its output is PR-controlled text (branch names,
#      ledger fields), so it is printed inside a ::stop-commands:: block with a random token, then
#      `warning: ` lines are re-emitted as ::warning:: and `note: ` lines as ::notice::.
#      An empty token fails the job. The job fails iff the binary exits non-zero.
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

head_sha=${PR_HEAD_SHA:-}
case $head_sha in
  '') echo "::error::PR_HEAD_SHA (the head commit the event names) must be set; refusing to validate an unknown head" >&2; exit 1 ;;
  *[!0-9a-fA-F]*) echo "::error::PR_HEAD_SHA is not a hex commit id; refusing to validate an unknown head" >&2; exit 1 ;;
esac
case ${#head_sha} in
  40 | 64) ;;
  *) echo "::error::PR_HEAD_SHA must be 40 or 64 hex characters; refusing to validate an unknown head" >&2; exit 1 ;;
esac
head_sha=$(printf '%s' "$head_sha" | tr 'A-F' 'a-f')
attempts=5
delay=${TALLY_CHECK_RETRY_DELAY:-5}
case $delay in '' | *[!0-9]*) delay=5 ;; esac

n=0
while :; do
  n=$((n + 1))
  if ! git fetch --quiet --no-tags "$remote" "+refs/pull/$pr/merge:refs/tally-check/merge" 2>/dev/null; then
    if listed=$(git ls-remote "$remote" "refs/pull/$pr/merge" 2>/dev/null); then
      if [ -z "$listed" ]; then
        echo "::error title=tally-check::refs/pull/$pr/merge does not exist: the PR probably has merge conflicts with its base. Resolve them so the check can run; it will not pass without a merge commit to read." >&2
      else
        echo "::error title=tally-check::could not fetch refs/pull/$pr/merge although it exists on the remote. Re-run the job." >&2
      fi
    else
      echo "::error title=tally-check::could not fetch refs/pull/$pr/merge: the remote was unreachable or refused access (network or auth), not a missing ref. Re-run the job." >&2
    fi
    exit 1
  fi
  head=$(git rev-parse --verify --quiet "refs/tally-check/merge^{commit}") ||
    { echo "::error::refs/pull/$pr/merge is not a commit" >&2; exit 1; }
  merged_head=$(git rev-parse --verify --quiet "$head^2^{commit}") ||
    { echo "::error::refs/pull/$pr/merge is not a merge commit; refusing to guess a base" >&2; exit 1; }
  [ "$merged_head" = "$head_sha" ] && break
  if [ "$n" -ge "$attempts" ]; then
    echo "::error title=tally-check::refs/pull/$pr/merge is stale: it merges $merged_head but the pull request head is $head_sha, after $attempts fetches. The check will not pass for a head it did not read. Re-run the job." >&2
    exit 1
  fi
  sleep "$delay"
done
base=$(git rev-parse --verify --quiet "$head^1^{commit}") ||
  { echo "::error::the merge commit has no first parent" >&2; exit 1; }

changed=$(git diff --name-only "$base" "$head" -- .tally/)
if [ -z "$changed" ]; then
  echo "::notice title=tally-check::no .tally/ file changed in this pull request"
  exit 0
fi

out=$(mktemp)
trap 'rm -f "$out"' EXIT
token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
[ -n "$token" ] || { echo "::error::could not generate a stop-commands token; refusing to print PR-controlled output" >&2; exit 1; }
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
