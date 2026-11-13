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
2. Resolves "latest" to a concrete version by asking the GitHub API (see
   [Which release is the latest](#which-release-is-the-latest)). Everything after that is
   downloaded from that one version.
3. Downloads `SHA256SUMS` and `branch-tally-<version>-<target>.zip`, and checks the archive's
   SHA-256 against its line in `SHA256SUMS`. **A mismatch, or an archive missing from
   `SHA256SUMS`, stops the install and nothing is written.** This is an integrity check: it
   catches a corrupt or truncated download, and it prints `Checksum OK (an integrity check; it
   does not prove who built the archive)`.
4. Checks where the archive came from, using its build provenance attestation, if a tool for that
   is installed. See [Checking where the archive came from](#checking-where-the-archive-came-from).
5. Extracts `branch-tally.exe` and moves it over any existing copy in the install directory in one
   step. If the old binary is running, the install fails with `cannot replace … (is branch-tally
   running?)`.
6. Runs the installed binary's `--version` to prove it works.
7. If the install directory is not on your `PATH`, prints two commands: one that adds it for your
   user (new terminals pick it up), and one that adds it to the current terminal only. It never
   edits your `PATH` itself.

## Which release is the latest

Without `-Version`, the installer asks the GitHub API
(`https://api.github.com/repos/IAmSkweetis/branch-tally/releases/latest`) which release is the
latest, even when a mirror is set. A mirror serving an older release as "latest" could otherwise
roll you back to a version with a known bug.

If the API can't be reached, is rate limited, or answers with something that isn't a `vX.Y.Z`
tag, the installer falls back to the mirror's `latest/download/SHA256SUMS`. It warns on stderr
(`install.ps1: warning: could not read the latest release from …`) that "latest" came from the
mirror and was not confirmed by GitHub, so an older release could be served, and suggests
`install.ps1 -Version vX.Y.Z`. The fallback goes ahead even with `-RequireAttestation`, because the
archive's checksum and provenance are still checked. To rule it out, pin `-Version`. That is also
the way to install from a mirror that can't reach `api.github.com`.

The API allows 60 requests an hour without a token. If `GH_TOKEN` is set, or else
`GITHUB_TOKEN`, the installer sends it as a bearer token to the API lookup, and to nothing else.
It never prints the token.

The protection against being rolled back has limits. It doesn't hold in the fallback above, nor
against someone who can intercept `api.github.com` itself, for example a proxy whose certificate
your machine trusts.

## Checking where the archive came from

`SHA256SUMS` comes from the same place as the archive, so a matching checksum does not prove who
built the archive: anyone able to replace one could replace both. Proof of origin is the release's
build provenance attestation. After the checksum, the installer verifies it with the first of these
that is available:

1. **The [GitHub CLI](https://cli.github.com/) (`gh`), logged in** (`gh auth status` succeeds). It
   runs:

   ```powershell
   gh attestation verify <archive> --repo IAmSkweetis/branch-tally `
     --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml `
     --source-ref refs/heads/main `
     --cert-oidc-issuer https://token.actions.githubusercontent.com
   ```

   That accepts only an attestation made by this repository's release workflow, running from
   `main`. On success it prints `Provenance verified with gh: <archive> was built by
   IAmSkweetis/branch-tally`.
2. **[cosign](https://docs.sigstore.dev/).** It downloads the release asset
   `provenance.sigstore.json` and runs `cosign verify-blob-attestation` on the archive with that
   bundle, requiring the exact signer
   `https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main` and
   the issuer `https://token.actions.githubusercontent.com`.
3. **Neither.** It writes `install.ps1: warning: provenance NOT verified: …` to stderr, prints the
   `gh attestation verify` command to run yourself, and installs. `-RequireAttestation` turns this
   warning into an error, and nothing is installed.

A check that runs and fails always stops the install, and nothing is written. Releases published
before `provenance.sigstore.json` existed don't have that asset, so on those the `cosign` route
falls through to the warning; `gh` still works for them.

To check a download yourself, including its signature, see
[Download a release by hand](manual-download.md).

## Options

Every option has a parameter and an environment variable. A parameter overrides its variable.

| Parameter | Environment variable | Default | Meaning |
| --------- | -------------------- | ------- | ------- |
| `-Version <vX.Y.Z>` | `BRANCH_TALLY_INSTALL_VERSION` | `latest` | Install this release instead of the latest. The leading `v` is optional. |
| `-To <dir>` | `BRANCH_TALLY_INSTALL_DIR` | `%USERPROFILE%\.local\bin` | Install into this directory. It is created if missing. |
| `-RequireAttestation` | `BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION` | off | Fail unless provenance was verified with `gh` or `cosign`. The variable turns it on when set to `1`, `true` or `yes`. |
| none | `BRANCH_TALLY_INSTALL_BASE_URL` | `https://github.com/IAmSkweetis/branch-tally/releases` | Download from a mirror with the same layout (`<base>/download/<tag>/…`, and `<base>/latest/download/SHA256SUMS` for the fallback). Only `https://` and `file://` URLs are accepted. |
| none | `BRANCH_TALLY_INSTALL_API_URL` | `https://api.github.com` | Where "latest" is looked up. Only `https://` and `file://` URLs are accepted. It is trusted exactly as much as the mirror. |

A version must be of the form `vMAJOR.MINOR.PATCH`; anything else is an error.

When `BRANCH_TALLY_INSTALL_BASE_URL` is set to anything other than the default, the installer
prints `Using release mirror: <url>` first, so you can see where it is downloading from. Likewise,
a non-default `BRANCH_TALLY_INSTALL_API_URL` prints `Using release API: <url>`. A mirror only
changes the source; the checksum and provenance checks are the same, and "latest" still comes from
the API (see [Which release is the latest](#which-release-is-the-latest)).

Every download follows redirects one hop at a time, at most 5, and refuses any hop that isn't
`https://`, on Windows PowerShell 5.1 and PowerShell 7 alike. A token is not sent on a redirect to
another host.

`irm | iex` cannot pass parameters, so with the one-liner use the environment variables:

```powershell
$env:BRANCH_TALLY_INSTALL_VERSION = 'v0.6.1'
$env:BRANCH_TALLY_INSTALL_DIR = 'C:\tools\bin'
$env:BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION = '1'
irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex
```

To use the parameters, download `install.ps1` from a release and run it as a file:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Version v0.6.1 -To C:\tools\bin -RequireAttestation
```

`Get-Help .\install.ps1` prints the script's own description of the options.

## Errors

The installer writes `install.ps1: error: <reason>` to stderr when, among other things:

- a version is malformed;
- `BRANCH_TALLY_INSTALL_BASE_URL` or `BRANCH_TALLY_INSTALL_API_URL` does not start with `https://`
  or `file://`;
- it is run on something other than Windows (use the [Linux and macOS](linux-macos.md) installer
  there), or the architecture is not x64 or arm64;
- the release has no build for your target, or a download hits a non-`https://` redirect or more
  than 5 redirects;
- a download fails, the checksum does not match, or the archive is not listed in `SHA256SUMS`;
- a provenance check with `gh` or `cosign` fails, or none could run and `-RequireAttestation` is
  set;
- the existing binary cannot be replaced;
- the installed binary does not run.

Run as a file, it exits with status 1. Under `irm | iex` it does not exit, because that would close
your terminal; it sets `$LASTEXITCODE` to 1 instead.
