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
| Download an archive, check `SHA256SUMS` and the provenance attestation yourself | [Download a release by hand](manual-download.md) |
| Compile it with cargo | [Build from source](from-source.md) |

The installers download the archive for your platform from GitHub, check it against the release's
`SHA256SUMS` and copy the binary into place. They never edit your shell startup files or your
`PATH`.

Package managers (crates.io, Homebrew, npm) are not available yet; they are tracked on the
[roadmap](https://github.com/IAmSkweetis/branch-tally/issues/16).

## Updating

A binary installed by an installer or by hand updates itself:

```sh
branch-tally update            # ask, then install the latest release
branch-tally update --check    # only say whether a different version is available
```

It verifies the download against the release's `SHA256SUMS` like the installers do, and asks before
replacing anything unless you pass `--yes`. `--version <vX.Y.Z>` installs a particular release,
older ones included. The [CLI reference](../cli.md#branch-tally-update) covers every option.

`update` refuses a [build from source](from-source.md) and a binary in a directory it cannot write
to. Running the installer again also works; it replaces the existing binary in one step.

## Uninstalling

To uninstall, delete the binary (`~/.local/bin/branch-tally`, or
`%USERPROFILE%\.local\bin\branch-tally.exe`, unless you chose another directory). branch-tally
also may have written these files, which you can delete:

- the user config file, if you ran `config init` (see [Config reference](../config.md#location));
- the price cache, if you ran `prices update` (see
  [CLI reference](../cli.md#branch-tally-prices-update)).
