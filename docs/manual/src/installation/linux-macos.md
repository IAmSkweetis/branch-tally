# Linux and macOS

`install.sh` installs the latest release for Linux (x86_64, aarch64) and macOS (Intel, Apple
silicon):

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
```

On success it prints the version and target it is installing, the installed path, and the
installed binary's `--version`:

```text
Installing branch-tally v0.6.1 for x86_64-unknown-linux-musl
Installed /home/you/.local/bin/branch-tally
branch-tally 0.6.1
```

## What it does

1. Works out the target from `uname`. Linux always gets the static musl build. On macOS, a shell
   running under Rosetta on an Apple silicon Mac still gets the Apple silicon build.
2. Resolves "latest" to a concrete version by reading the release's `SHA256SUMS`, without calling
   the GitHub API. Everything after that is downloaded from that one version, so a release
   published mid-install can't mix two versions.
3. Downloads `SHA256SUMS` and `branch-tally-<version>-<target>.tar.gz`, and checks the archive's
   SHA-256 against its line in `SHA256SUMS`. **A mismatch, or an archive missing from
   `SHA256SUMS`, stops the install and nothing is written.**
4. Extracts the binary and moves it over any existing `branch-tally` in the install directory in a
   single rename.
5. Runs the installed binary's `--version` to prove it works.
6. If the install directory is not on your `PATH`, prints the line to add to your shell profile.
   It never edits a startup file.

It needs only tools a stock system has: `curl` or `wget`, `sha256sum` or `shasum`, and `tar`.

The installer does not verify the provenance attestation. To do that, see
[Download a release by hand](manual-download.md).

## Options

Every option has a flag and an environment variable. A flag overrides its variable.

| Flag | Environment variable | Default | Meaning |
| ---- | -------------------- | ------- | ------- |
| `--version <vX.Y.Z>` | `BRANCH_TALLY_INSTALL_VERSION` | `latest` | Install this release instead of the latest. The leading `v` is optional. |
| `--to <dir>` | `BRANCH_TALLY_INSTALL_DIR` | `$HOME/.local/bin` | Install into this directory. It is created if missing. |
| none | `BRANCH_TALLY_INSTALL_BASE_URL` | `https://github.com/IAmSkweetis/branch-tally/releases` | Download from a mirror with the same layout (`<base>/latest/download/…` and `<base>/download/<tag>/…`). |
| `-h`, `--help` | none | | Print the options and exit. |

A version must be of the form `vMAJOR.MINOR.PATCH`; anything else is an error.

To pass flags through the pipe, use `sh -s --`:

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh \
  | sh -s -- --version v0.6.1 --to ~/bin
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
- the OS or architecture is not one of the supported targets (from a Windows shell such as Git
  Bash, it tells you to use the [PowerShell installer](windows.md) instead);
- `curl`/`wget`, `sha256sum`/`shasum` or `tar` is missing;
- the latest release has no build for your target;
- a download fails, the checksum does not match, or the archive is not listed in `SHA256SUMS`;
- the install directory cannot be created or written;
- the installed binary does not run.

`HOME` must be set unless you pass `--to`.
