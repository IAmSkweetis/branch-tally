# Download a release by hand

Every release on the [releases page](https://github.com/IAmSkweetis/branch-tally/releases)
attaches:

- six archives, one per target, named `branch-tally-<vX.Y.Z>-<target>.tar.gz` (`.zip` for
  Windows). The targets are listed on the [Installation](./) page;
- `install.sh` and `install.ps1`;
- `SHA256SUMS`, with one SHA-256 line for each archive and each installer;
- `SHA256SUMS.minisig`, a [minisign](https://jedisct1.github.io/minisign/) signature of
  `SHA256SUMS` made with the maintainer's release key;
- `provenance.sigstore.json`, a Sigstore
  [build provenance attestation](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations)
  covering the archives, the installers, `SHA256SUMS` and `SHA256SUMS.minisig`.

The files prove different things:

| File | What it proves |
| ---- | -------------- |
| `SHA256SUMS` | Integrity only: the download is complete and not corrupted. It comes from the same place as the archive, so whoever can replace one can replace both. |
| `SHA256SUMS.minisig` | Origin: the maintainer's key signed this `SHA256SUMS`, for this release. |
| The attestation | Origin: this repository's release workflow built the file from the tagged commit. |

Releases published before signing began have no `SHA256SUMS.minisig` and no
`provenance.sigstore.json`. Their archives and installers do have attestations, which
`gh attestation verify` (step 4) checks.

The commands below use a shell variable for the release, so set it first. For example, for Linux
x86_64:

```sh
tag=vX.Y.Z    # the release you want, for example the latest on the releases page
target=x86_64-unknown-linux-musl
```

## 1. Download

Download the archive for your platform, `SHA256SUMS` and `SHA256SUMS.minisig`:

```sh
base=https://github.com/IAmSkweetis/branch-tally/releases/download/$tag
curl -fsSLO "$base/branch-tally-$tag-$target.tar.gz"
curl -fsSLO "$base/SHA256SUMS"
curl -fsSLO "$base/SHA256SUMS.minisig"
```

## 2. Check the signature

This step needs [minisign](https://jedisct1.github.io/minisign/). Take the public key from the
repository, not from the release page, since a release page someone has tampered with could carry
a key of their own. It is the file
[`.github/minisign.pub`](https://github.com/IAmSkweetis/branch-tally/blob/main/.github/minisign.pub),
with the key id `D9F252276FF0E4D0`:

```sh
curl -fsSLo minisign.pub https://raw.githubusercontent.com/IAmSkweetis/branch-tally/main/.github/minisign.pub
minisign -Vm SHA256SUMS -p minisign.pub
```

minisign reads the signature from `SHA256SUMS.minisig` next to `SHA256SUMS`. A good signature
prints:

```text
Signature and comment signature verified
Trusted comment: branch-tally vX.Y.Z SHA256SUMS
```

**Check that the tag in the trusted comment is the release you downloaded.** The comment is signed
too, so it can't be altered, and it stops someone serving an older release's signed files in place
of the one you asked for. A signature made by any other key fails with a key id mismatch, and a
`SHA256SUMS` that has been changed fails with `Signature verification failed`. Both exit non-zero.

Instead of the file, you can pass the key itself with `-P`:

```sh
minisign -Vm SHA256SUMS -P RWTQ5PBvJ1Ly2e0dZqwbC5mduW++vTIp1vIHDe+C8z7/4VNNvoU30mmr
```

## 3. Check the checksum

The signature covers `SHA256SUMS`. The checksum then ties the archive to that file.

On Linux:

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

`--ignore-missing` skips the lines for the files you did not download. On macOS, check only the
archive's line:

```sh
grep "$target.tar.gz" SHA256SUMS | shasum -a 256 -c
```

Either prints `<archive name>: OK` for a good download.

On Windows, in PowerShell, compare the hash with the archive's line in `SHA256SUMS`:

```powershell
$tag = 'vX.Y.Z'
(Get-FileHash ".\branch-tally-$tag-x86_64-pc-windows-msvc.zip" -Algorithm SHA256).Hash.ToLower()
Select-String 'x86_64-pc-windows-msvc.zip' .\SHA256SUMS
```

On its own, a checksum taken from the same release only proves the download is complete and
uncorrupted. Its value comes from the signature in step 2, or the attestation in step 4.

## 4. Check the attestation

The attestation is a second, independent proof of origin: it shows the file was built by this
repository's release workflow. With the [GitHub CLI](https://cli.github.com/) installed and signed
in:

```sh
gh attestation verify "branch-tally-$tag-$target.tar.gz" \
  --repo IAmSkweetis/branch-tally \
  --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
  --source-ref refs/heads/main
```

`--repo` on its own would accept an attestation made by any workflow in the repository, so the
command also pins the release workflow and the branch it runs from. It exits 0 when the
attestation is valid and matches, and non-zero otherwise. The same command works for `install.sh`
and `install.ps1`, and on a signed release for `SHA256SUMS` and `SHA256SUMS.minisig` too. This is
the command the installers run (they also pass
`--cert-oidc-issuer https://token.actions.githubusercontent.com`, which is `gh`'s default).

Without `gh`, [cosign](https://docs.sigstore.dev/) can check the archive against
`provenance.sigstore.json`. This is the check the installers run when `gh` is not available:

```sh
curl -fsSLO "$base/provenance.sigstore.json"
cosign verify-blob-attestation --bundle provenance.sigstore.json --new-bundle-format \
  --type slsaprovenance1 \
  --certificate-identity https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "branch-tally-$tag-$target.tar.gz"
```

## 5. Install

Each archive holds one directory, `branch-tally-<vX.Y.Z>-<target>/`, containing the binary
(`branch-tally`, or `branch-tally.exe` on Windows), `LICENSE` and `README.md`.

```sh
tar -xzf "branch-tally-$tag-$target.tar.gz"
mkdir -p ~/.local/bin
cp "branch-tally-$tag-$target/branch-tally" ~/.local/bin/
branch-tally --version
```

On Windows, extract the `.zip` (for example with `Expand-Archive`) and copy `branch-tally.exe` to a
directory on your `PATH`.

Any directory on your `PATH` will do; `~/.local/bin` is only the default the installers use.

A release from before signing can only be installed this way or with an installer.
[`branch-tally update`](../cli.md#branch-tally-update) refuses it, because it has no signature to
verify.
