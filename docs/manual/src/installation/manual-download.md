# Download a release by hand

Every release on the [releases page](https://github.com/IAmSkweetis/branch-tally/releases)
attaches:

- six archives, one per target, named `branch-tally-<vX.Y.Z>-<target>.tar.gz` (`.zip` for
  Windows). The targets are listed on the [Installation](README.md) page;
- `install.sh` and `install.ps1`;
- `SHA256SUMS`, with one SHA-256 line for each archive and each installer.

Every archive and installer also has a GitHub
[build provenance attestation](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations),
which proves it was built by this repository's release workflow from the tagged commit.

## 1. Download

Pick the archive for your platform and download it together with `SHA256SUMS`. For example, for
v0.6.1 on Linux x86_64:

```sh
curl -fsSLO https://github.com/IAmSkweetis/branch-tally/releases/download/v0.6.1/branch-tally-v0.6.1-x86_64-unknown-linux-musl.tar.gz
curl -fsSLO https://github.com/IAmSkweetis/branch-tally/releases/download/v0.6.1/SHA256SUMS
```

## 2. Check the checksum

On Linux:

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

`--ignore-missing` skips the lines for the files you did not download. On macOS, check only the
archive's line:

```sh
grep 'aarch64-apple-darwin.tar.gz' SHA256SUMS | shasum -a 256 -c
```

Either prints `<archive name>: OK` for a good download.

On Windows, in PowerShell, compare the hash with the archive's line in `SHA256SUMS`:

```powershell
(Get-FileHash .\branch-tally-v0.6.1-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash.ToLower()
Select-String 'x86_64-pc-windows-msvc.zip' .\SHA256SUMS
```

A checksum taken from the same release only proves the download is complete and uncorrupted. For
proof of where it came from, check the attestation.

## 3. Check the attestation

With the [GitHub CLI](https://cli.github.com/) installed and signed in:

```sh
gh attestation verify branch-tally-v0.6.1-x86_64-unknown-linux-musl.tar.gz --repo IAmSkweetis/branch-tally
```

It exits 0 when the archive's attestation is valid and was issued for this repository, and
non-zero otherwise. The same command works for `install.sh` and `install.ps1`.

## 4. Install

Each archive holds one directory, `branch-tally-<vX.Y.Z>-<target>/`, containing the binary
(`branch-tally`, or `branch-tally.exe` on Windows), `LICENSE` and `README.md`.

```sh
tar -xzf branch-tally-v0.6.1-x86_64-unknown-linux-musl.tar.gz
mkdir -p ~/.local/bin
cp branch-tally-v0.6.1-x86_64-unknown-linux-musl/branch-tally ~/.local/bin/
branch-tally --version
```

On Windows, extract the `.zip` (for example with `Expand-Archive`) and copy `branch-tally.exe` to a
directory on your `PATH`.

Any directory on your `PATH` will do; `~/.local/bin` is only the default the installers use.
