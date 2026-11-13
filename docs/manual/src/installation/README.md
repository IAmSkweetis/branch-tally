# Installation

branch-tally is a single static binary. Every release on the
[releases page](https://github.com/IAmSkweetis/branch-tally/releases) ships it prebuilt for six
targets:

| Platform | Target | Archive |
| -------- | ------ | ------- |
| Linux x86_64 | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Linux aarch64 | `aarch64-unknown-linux-musl` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS Apple silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows arm64 | `aarch64-pc-windows-msvc` | `.zip` |

The Linux builds are statically linked against musl, so they run on any distribution, whatever its
C library.

Choose a method:

| Method | Page |
| ------ | ---- |
| One-line installer for Linux and macOS (`install.sh`) | [Linux and macOS](linux-macos.md) |
| One-line installer for Windows (`install.ps1`) | [Windows](windows.md) |
| Download an archive, check its signature, `SHA256SUMS` and the provenance attestation yourself | [Download a release by hand](manual-download.md) |
| Compile it with cargo | [Build from source](from-source.md) |

The installers download the archive for your platform from GitHub, check it against the release's
`SHA256SUMS` and copy the binary into place. That checksum is an integrity check: it catches a
corrupt or truncated download, but it doesn't prove who built the archive. When the GitHub CLI
(`gh`) or cosign is installed, the installers also verify the archive's build provenance
attestation, which does. Without either, they warn that provenance was not verified, and install
anyway unless you ask them not to. They never edit your shell startup files or your `PATH`.

Package managers (crates.io, Homebrew, npm) are not available yet; they are tracked on the
[roadmap](https://github.com/IAmSkweetis/branch-tally/milestone/1).

## Updating

A binary installed by an installer or by hand updates itself:

```sh
branch-tally update            # ask, then install the latest release
branch-tally update --check    # only say whether a different version is available
```

It asks the GitHub API which release is the latest, even when a mirror is set, and fails rather
than guess if the API can't be reached; pass `--version` then. Without `--version` it never
installs a release older than the one running.

Before it trusts anything it downloads, it checks the release's minisign signature
(`SHA256SUMS.minisig`) against the release key built into the binary. That signature is its proof
of origin. Only then does it check the archive against `SHA256SUMS`. It asks before replacing
anything unless you pass `--yes`. `--version <vX.Y.Z>` installs a particular release, older ones
included, as long as that release is signed. The [CLI reference](../cli.md#branch-tally-update)
covers every option.

Releases published before signing began have no signature, so `update` refuses them, including
through `--version`, and installs nothing. Install one of those with an installer or
[by hand](manual-download.md) instead.

`update` refuses a [build from source](from-source.md) and a binary in a directory it cannot write
to. Running the installer again also works; it replaces the existing binary in one step.

## Uninstalling

To uninstall, delete the binary (`~/.local/bin/branch-tally`, or
`%USERPROFILE%\.local\bin\branch-tally.exe`, unless you chose another directory). branch-tally
also may have written these files, which you can delete:

- the user config file, if you ran `config init` (see [Config reference](../config.md#location));
- the price cache, if you ran `prices update` (see
  [CLI reference](../cli.md#branch-tally-prices-update)).
