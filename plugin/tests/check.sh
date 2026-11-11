#!/usr/bin/env sh
# Checks the shipped plugin and marketplace manifests. Run from the repository root.
#   plugin/tests/check.sh [path-to-branch-tally-binary]
# 1. `claude plugin validate` passes on the marketplace and on each plugin.
# 2. Every `branch-tally <subcommand>` named in a hook command is a real subcommand of the binary
#    (skipped, loudly, when no binary is given).
# 3. Every hook command ends in `; exit 0`, so no failure can reach Claude Code as a blocking
#    exit 2 (clap usage errors exit 2).
set -u
fail=0
bin="${1:-}"
err() { echo "FAIL: $*" >&2; fail=1; }

claude plugin validate . || err "marketplace did not validate"
[ -d plugin/capture-hooks ] || err "plugin/capture-hooks missing"
[ -d plugin/record-hooks ] || err "plugin/record-hooks missing"
for dir in plugin/*-hooks; do
  [ -d "$dir" ] || continue
  claude plugin validate "$dir" || err "$dir did not validate"
  f="$dir/hooks/hooks.json"
  [ -f "$f" ] || { err "$dir has no hooks/hooks.json"; continue; }
  for line in $(grep -o '"command": *"[^"]*"' "$f" | tr ' ' '~'); do
    case "$line" in
      *';~exit~0"') ;;
      *) err "$f: hook command can exit non-zero: $line" ;;
    esac
  done
  if [ -n "$bin" ]; then
    for sub in $(grep -o 'branch-tally [a-z-]*' "$f" | awk '{print $2}' | sort -u); do
      "$bin" "$sub" --help >/dev/null 2>&1 || err "$f names unknown subcommand: $sub"
    done
  else
    echo "SKIP: no binary given, subcommand names not checked"
  fi
done
exit "$fail"
