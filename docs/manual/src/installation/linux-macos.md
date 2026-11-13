# Linux and macOS

`install.sh` installs the latest release for Linux (x86_64, aarch64) and macOS (Intel, Apple
silicon):

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
```

On success it prints the version and target it is installing, the installed path, and the
installed binary's `--version`:

```text
Installing branch-tally v0.10.0 for x86_64-unknown-linux-musl
Checksum OK (an integrity check; it does not prove who built the archive)
Provenance verified with gh: branch-tally-v0.10.0-x86_64-unknown-linux-musl.tar.gz was built by IAmSkweetis/branch-tally
Installed /home/you/.local/bin/branch-tally
branch-tally 0.10.0
```

The `Provenance verified` line appears only when a provenance tool was available. See
[Checking where the archive came from](#checking-where-the-archive-came-from).

## What it does

1. Works out the target from `uname`. Linux always gets the static musl build. On macOS, a shell
   running under Rosetta on an Apple silicon Mac still gets the Apple silicon build.
2. Resolves "latest" to a concrete version by reading the release's `SHA256SUMS`, without calling
   the GitHub API. Everything after that is downloaded from that one version, so a release
   published mid-install can't mix two versions.
3. Downloads `SHA256SUMS` and `branch-tally-<version>-<target>.tar.gz`, and checks the archive's
   SHA-256 against its line in `SHA256SUMS`. **A mismatch, or an archive missing from
   `SHA256SUMS`, stops the install and nothing is written.** This is an integrity check: it
   catches a corrupt or truncated download.
4. Checks where the archive came from, using its build provenance attestation, if a tool for that
   is installed. See [Checking where the archive came from](#checking-where-the-archive-came-from).
5. Extracts the binary and moves it over any existing `branch-tally` in the install directory in a
   single rename.
6. Runs the installed binary's `--version` to prove it works.
7. If the install directory is not on your `PATH`, prints the line to add to your shell profile.
   It never edits a startup file.

It needs only tools a stock system has: `curl` or `wget`, `sha256sum` or `shasum`, and `tar`.
The [GitHub CLI](https://cli.github.com/) (`gh`) or [cosign](https://docs.sigstore.dev/) is
optional, and lets it check provenance.

With `curl`, every download must use HTTPS, redirects included, and TLS 1.2 or newer. With `wget`,
it passes `--https-only` when that `wget` supports it.

## Checking where the archive came from

`SHA256SUMS` comes from the same place as the archive, so a matching checksum does not prove who
built the archive: anyone able to replace one could replace both. Proof of origin is the release's
build provenance attestation. After the checksum, the installer verifies it with the first of these
that is available:

1. **`gh`, logged in** (`gh auth status` succeeds). It runs:

   ```sh
   gh attestation verify <archive> --repo IAmSkweetis/branch-tally \
     --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
     --source-ref refs/heads/main \
     --cert-oidc-issuer https://token.actions.githubusercontent.com
   ```

   That accepts only an attestation made by this repository's release workflow, running from
   `main`.
2. **`cosign`.** It downloads the release asset `provenance.sigstore.json` and runs
   `cosign verify-blob-attestation` on the archive with that bundle, requiring the exact signer
   `https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main`
   and the issuer `https://token.actions.githubusercontent.com`.
3. **Neither.** It warns on stderr, then installs:

   ```text
   install.sh: warning: provenance NOT verified: neither gh (logged in) nor cosign is available.
   ```

   The lines after it say that the checksum is an integrity check only, and give the URL of the
   archive and the `gh attestation verify` command to check it yourself.

   `--require-attestation` turns this warning into an error, and nothing is installed.

A check that runs and fails always stops the install, and nothing is written. Releases published
before `provenance.sigstore.json` existed don't have that asset, so on those the `cosign` route
falls through to the warning; `gh` still works for them.

To check a download yourself, including its signature, see
[Download a release by hand](manual-download.md).

## Options

Every option has a flag and an environment variable. A flag overrides its variable.

| Flag | Environment variable | Default | Meaning |
| ---- | -------------------- | ------- | ------- |
| `--version <vX.Y.Z>` | `BRANCH_TALLY_INSTALL_VERSION` | `latest` | Install this release instead of the latest. The leading `v` is optional. |
| `--to <dir>` | `BRANCH_TALLY_INSTALL_DIR` | `$HOME/.local/bin` | Install into this directory. It is created if missing. |
| `--require-attestation` | `BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION` | off | Fail unless provenance was verified with `gh` or `cosign`. The variable turns it on when set to `1`, `true` or `yes`. |
| none | `BRANCH_TALLY_INSTALL_BASE_URL` | `https://github.com/IAmSkweetis/branch-tally/releases` | Download from a mirror with the same layout (`<base>/latest/download/…` and `<base>/download/<tag>/…`). Only `https://` and `file://` URLs are accepted. |
| `-h`, `--help` | none | | Print the options and exit. |

A version must be of the form `vMAJOR.MINOR.PATCH`; anything else is an error.

When `BRANCH_TALLY_INSTALL_BASE_URL` is set to anything other than the default, the installer
prints `Using release mirror: <url>` first, so you can see where it is downloading from. A mirror
only changes the source; the checksum and provenance checks are the same.

To pass flags through the pipe, use `sh -s --`:

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh \
  | sh -s -- --version v0.6.1 --to ~/bin
```

To refuse an install whose provenance can't be checked:

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh \
  | sh -s -- --require-attestation
```

Or set the variables:

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh \
  | BRANCH_TALLY_INSTALL_DIR="$HOME/bin" sh
```

You can also download `install.sh` from a release, read it, and run it as a file:
`sh install.sh --help`.

## Errors

The installer prints `install.sh: error: <reason>` and exits with status 1 when, among other
things:

- an option is unknown or a version is malformed;
- `BRANCH_TALLY_INSTALL_BASE_URL` does not start with `https://` or `file://`;
- the OS or architecture is not one of the supported targets (from a Windows shell such as Git
  Bash, it tells you to use the [PowerShell installer](windows.md) instead);
- `curl`/`wget`, `sha256sum`/`shasum` or `tar` is missing;
- the latest release has no build for your target;
- a download fails, the checksum does not match, or the archive is not listed in `SHA256SUMS`;
- a provenance check with `gh` or `cosign` fails, or none could run and `--require-attestation` is
  set;
- the install directory cannot be created or written;
- the installed binary does not run.

`HOME` must be set unless you pass `--to`.
