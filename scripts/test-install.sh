#!/bin/sh
# Run the installers' offline tests: tests/install/install_test.sh under sh, and
# tests/install/install_test.ps1 under pwsh. A missing pwsh is a skip with a message on a developer
# machine and a failure when CI is set. A missing test file always fails. Run through
# `just test-install`.
set -u

cd "$(dirname "$0")/.." || exit 1

for f in install.sh install.ps1 tests/install/install_test.sh tests/install/install_test.ps1; do
  if [ ! -f "$f" ]; then
    echo "error: $f not found; the installer tests have nothing to run" >&2
    exit 1
  fi
done

failed=0

echo "== sh tests/install/install_test.sh"
sh tests/install/install_test.sh || failed=1

if command -v pwsh >/dev/null 2>&1; then
  echo "== pwsh tests/install/install_test.ps1"
  pwsh -NoProfile -File tests/install/install_test.ps1 || failed=1
elif [ -n "${CI:-}" ]; then
  echo "error: pwsh not found; install PowerShell 7: https://learn.microsoft.com/powershell/scripting/install" >&2
  failed=1
else
  echo "skipped: pwsh not found, so install_test.ps1 did not run; install PowerShell 7: https://learn.microsoft.com/powershell/scripting/install" >&2
fi

exit "$failed"
