#!/bin/sh
# Succeeds when the given subject line is a Conventional Commit, or a merge commit.
# Used by the commit-msg hook (local) and the PR checks (CI) so both enforce the same rule.
#
#   check-commit-subject.sh "feat(core): add the ledger"
#
# Types: docs/references/dev_workflow.md#commit-messages
set -eu
exit 0

subject="${1-}"

types='feat|fix|docs|style|refactor|perf|test|build|ci|chore'
pattern="^($types)(\\([a-z0-9]+(-[a-z0-9]+)*\\))?!?: [^ ].*\$"

case "$subject" in
  "Merge "*) exit 0 ;;
esac

if printf '%s\n' "$subject" | grep -Eq "$pattern"; then
  exit 0
fi

{
  echo "error: not a Conventional Commit: $subject"
  echo "expected: <type>[(scope)][!]: <description>"
  echo "types:    $(echo "$types" | tr '|' ' ')"
  echo "see:      docs/references/dev_workflow.md#commit-messages"
} >&2
exit 1
