# Windows

`install.ps1` installs the latest release for Windows on x64 or arm64. Run it from PowerShell:

```powershell
irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex
```

If your execution policy refuses it, run it through a one-off bypass:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex"
```

Windows PowerShell 5.1 and PowerShell 7 both work. The installer needs nothing beyond built-in
PowerShell.

## What it does

1. Picks the build for the machine's **OS** architecture, not the PowerShell process's, so a
   32-bit or emulated PowerShell on an arm64 machine still gets the arm64 build.
2. Resolves "latest" to a concrete version by reading the release's `SHA256SUMS`, without calling
   the GitHub API. Everything after that is downloaded from that one version.
3. Downloads `SHA256SUMS` and `branch-tally-<version>-<target>.zip`, and checks the archive's
   SHA-256 against its line in `SHA256SUMS`. **A mismatch, or an archive missing from
   `SHA256SUMS`, stops the install and nothing is written.**
4. Extracts `branch-tally.exe` and moves it over any existing copy in the install directory in one
   step. If the old binary is running, the install fails with `cannot replace … (is branch-tally
   running?)`.
5. Runs the installed binary's `--version` to prove it works.
6. If the install directory is not on your `PATH`, prints two commands: one that adds it for your
   user (new terminals pick it up), and one that adds it to the current terminal only. It never
   edits your `PATH` itself.

The installer does not verify the provenance attestation. To do that, see
[Download a release by hand](manual-download.md).

## Options

Every option has a parameter and an environment variable. A parameter overrides its variable.

| Parameter | Environment variable | Default | Meaning |
| --------- | -------------------- | ------- | ------- |
| `-Version <vX.Y.Z>` | `BRANCH_TALLY_INSTALL_VERSION` | `latest` | Install this release instead of the latest. The leading `v` is optional. |
| `-To <dir>` | `BRANCH_TALLY_INSTALL_DIR` | `%USERPROFILE%\.local\bin` | Install into this directory. It is created if missing. |
| none | `BRANCH_TALLY_INSTALL_BASE_URL` | `https://github.com/IAmSkweetis/branch-tally/releases` | Download from a mirror with the same layout. A `file://` URL also works. |

A version must be of the form `vMAJOR.MINOR.PATCH`; anything else is an error.

`irm | iex` cannot pass parameters, so with the one-liner use the environment variables:

```powershell
$env:BRANCH_TALLY_INSTALL_VERSION = 'v0.6.1'
$env:BRANCH_TALLY_INSTALL_DIR = 'C:\tools\bin'
irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex
```

To use the parameters, download `install.ps1` from a release and run it as a file:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Version v0.6.1 -To C:\tools\bin
```

`Get-Help .\install.ps1` prints the script's own description of the options.

## Errors

The installer writes `install.ps1: error: <reason>` to stderr when, among other things:

- a version is malformed;
- it is run on something other than Windows (use the [Linux and macOS](linux-macos.md) installer
  there), or the architecture is not x64 or arm64;
- the latest release has no build for your target;
- a download fails, the checksum does not match, or the archive is not listed in `SHA256SUMS`;
- the existing binary cannot be replaced;
- the installed binary does not run.

Run as a file, it exits with status 1. Under `irm | iex` it does not exit, because that would close
your terminal; it sets `$LASTEXITCODE` to 1 instead.
