#!/bin/sh
# Lint the installers: ShellCheck on install.sh and its test, PSScriptAnalyzer on the .ps1 files.
# A missing tool is a skip with a message on a developer machine, and a failure when CI is set, so
# the gate can't quietly pass in the one place it must run. A missing installer file always fails.
# Run through `just lint-install`.
set -u

cd "$(dirname "$0")/.." || exit 1

for f in install.sh install.ps1 tests/install/install_test.sh tests/install/install_test.ps1 tests/install/install_live_test.ps1; do
  if [ ! -f "$f" ]; then
    echo "error: $f not found; the installer gate has nothing to check" >&2
    exit 1
  fi
done

failed=0

# have <tool> <hint>: true when the tool exists. Otherwise an error in CI, a skip message elsewhere.
have() {
  command -v "$1" >/dev/null 2>&1 && return 0
  if [ -n "${CI:-}" ]; then
    echo "error: $1 not found; $2" >&2
    failed=1
  else
    echo "skipped: $1 not found, so this check did not run; $2" >&2
  fi
  return 1
}

if have shellcheck "install it: https://github.com/koalaman/shellcheck#installing"; then
  shellcheck install.sh tests/install/install_test.sh || failed=1
fi

if have pwsh "install PowerShell 7: https://learn.microsoft.com/powershell/scripting/install"; then
  pwsh -NoProfile -File scripts/lint-install.ps1 install.ps1 tests/install/install_test.ps1 tests/install/install_live_test.ps1 || failed=1
fi

exit "$failed"
