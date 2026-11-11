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

# install.sh does not target Windows (install.ps1 does), so on a Windows host, where `sh` is Git for
# Windows' MSYS shell, only the sh suite is skipped. The pwsh suite below still runs, and a missing
# pwsh is still a failure in CI, so this skip can't turn the gate into a false green.
windows=0
case "$(uname -s 2>/dev/null)" in MINGW* | MSYS* | CYGWIN*) windows=1 ;; esac
[ "${OS:-}" = "Windows_NT" ] && windows=1

if [ "$windows" -eq 1 ]; then
  echo "skipped: install.sh does not target Windows; install_test.ps1 covers it"
else
  echo "== sh tests/install/install_test.sh"
  sh tests/install/install_test.sh || failed=1
fi

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
