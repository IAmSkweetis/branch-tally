#!/bin/sh
# Succeeds when the given branch name is a Conventional Branch: <type>/<description>, where the
# description is lowercase a-z and 0-9 in hyphen-separated runs (dots also allowed under
# release/, for versions). `main` is the trunk and is accepted here; pushing to it is a separate
# rule, enforced by the pre-push hook and the ruleset.
#
# Dependabot's own branches (dependabot/<ecosystem>/<path>) are exempt: the bot names them and
# they cannot be renamed. Pass --allow-dependabot (CI does) to exempt them.
#
#   check-branch-name.sh [--allow-dependabot] "feat/issue-4-report-command"
#
# Rules: docs/references/dev_workflow.md#branch-names
set -eu

allow_dependabot=0
if [ "${1-}" = "--allow-dependabot" ]; then
  allow_dependabot=1
  shift
fi
branch="${1-}"

[ "$branch" = "main" ] && exit 0

if [ "$allow_dependabot" = 1 ]; then
  case "$branch" in
    dependabot/*) exit 0 ;;
  esac
fi

run='[a-z0-9]+(-[a-z0-9]+)*'
if printf '%s\n' "$branch" | grep -Eq "^(feat|feature|fix|bugfix|hotfix|chore)/$run\$" ||
  printf '%s\n' "$branch" | grep -Eq "^release/[a-z0-9.]+(-[a-z0-9.]+)*\$"; then
  exit 0
fi

{
  echo "error: not a Conventional Branch: $branch"
  echo "expected: <type>/<description> with type feat|feature|fix|bugfix|hotfix|chore|release,"
  echo "          description lowercase a-z 0-9, hyphen-separated (e.g. chore/issue-17-gates)"
  echo "see:      docs/references/dev_workflow.md#branch-names"
} >&2
exit 1
