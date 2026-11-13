# 0004. Verify release provenance with a minisign signature and Sigstore attestations

- **Status:** accepted
- **Date:** 2026-11-13
- **Issue:** #76 (Security Assessment F-04), with the installer items of #98 (F-06)

## Context

`branch-tally update`, `install.sh` and `install.ps1` fetched `SHA256SUMS` and the archive from the
same release, or the same mirror, and compared them. That catches a corrupted download but
authenticates nothing: whoever can replace the archive can replace the checksum file beside it.
The release pipeline signed build-provenance attestations for the archives, but nothing verified
them, and `SHA256SUMS` was not an attestation subject.

Two ways to authenticate a download were considered for `update`. A Sigstore bundle verified
in-binary needs no secret, but brings a large dependency tree (x509, TUF trust roots) into a binary
that reads private data and is meant to stay small. A detached signature whose public key is
compiled into the binary is small and works offline, at the cost of a signing secret in CI and a
key rotation story. The installers are scripts and can't embed a verifier, so they have to rely on
a tool the user already has.

## Decision

- **Releases sign `SHA256SUMS` with minisign** (Ed25519). The signature ships as the release asset
  `SHA256SUMS.minisig`. Its trusted comment is exactly `branch-tally <tag> SHA256SUMS` (for example
  `branch-tally v1.2.3 SHA256SUMS`), so a signature for one release can't be replayed as another.
  The maintainer generates the keypair offline. In the pipeline, the secret key exists only as a
  GitHub Actions secret, used only by `release.yml`'s publish job. No agent or test ever handles it,
  and tests use a throwaway test key.
- **`update` is strict.** It verifies `SHA256SUMS.minisig` against the public key compiled into the
  binary and checks the trusted comment names the tag being installed, before it trusts any
  checksum. A missing, invalid or mismatched signature installs nothing. A release from before
  signing is refused, including through `--version`, with a message pointing at the manual route.
  A mirror (`BRANCH_TALLY_INSTALL_BASE_URL`) is acceptable because trust comes from the signature,
  not the origin. A shipped binary has no runtime way to replace the public key.
- **`SHA256SUMS`, `SHA256SUMS.minisig` and the installers are attestation subjects** alongside the
  archives, and the attestation bundle ships as the release asset `provenance.sigstore.json`.
- **The installers verify provenance when they can and say so when they can't.** With `gh` on PATH
  and logged in, they run `gh attestation verify` pinned to this repository. Otherwise, with
  `cosign` on PATH, they verify `provenance.sigstore.json` with the signer identity pinned to this
  repository's `release.yml`. Otherwise they warn plainly that provenance was not verified, print
  the command to run, and install. `--require-attestation` (`-RequireAttestation`) turns that
  warning into a failure.
- **The installer hardening from F-06 lands here too:** curl restricted to HTTPS and TLS 1.2+, a
  scheme check on the mirror URL, the effective source printed when a mirror is set, `install.sh`
  wrapped in `main` so a truncated download can't run, and the tag read from `SHA256SUMS` validated
  as `vMAJOR.MINOR.PATCH`.
- **Wording.** The checksum is described as an integrity check everywhere. Only the signature and
  the attestation are described as proof of origin.
- **"Latest" comes from the GitHub API, not the mirror.** A signature proves a file is a genuine
  release, not that it is the newest one, so a mirror or proxy serving an older signed release as
  "latest" could roll users back to a known-vulnerable version. `update` and the installers
  resolve the latest tag from `api.github.com` even when a mirror is set, using `GH_TOKEN` or
  `GITHUB_TOKEN` when present. When the API can't be reached, `update` fails and asks for
  `--version`. The installers fall back to the mirror's `latest/download/SHA256SUMS` with a
  warning.
- **`update` never downgrades unless asked.** Without `--version`, it never installs a release
  older than the running binary, even with `--force`, which only reinstalls the same version. An
  explicit `--version` pin may still go older.
- **The post-release install check verifies provenance for real.** `install-check` runs the
  installers with `--require-attestation` through `gh` and through a pinned `cosign`, and verifies
  `SHA256SUMS.minisig` with minisign, so a wrong flag or pin fails CI instead of every user.

## Consequences

- Updating to, or pinning, a release cut before this change fails in `update`. Those releases can
  still be installed by hand.
- Losing or leaking the signing key needs a new binary with a new public key. Rotation is a
  release, and binaries that only know the old key can't verify the new one.
- `curl | sh` stays dependency-free. Without `gh` or `cosign` it installs unverified, and says so.
- Rollback protection is partial. It holds against a malicious mirror, but not against an attacker
  who can intercept `api.github.com` itself (for example a proxy with a trusted root), nor in
  the installers when the API is unreachable and they fall back. A signed "latest" pointer would
  close that and is not part of this decision.
- An air-gapped mirror can't resolve "latest" for `update`. Pin `--version` there.
- Binaries released before this change check only `SHA256SUMS` when they update, so the first
  update to a signed release is still same-origin. Every update after it is verified.
- F-05 (#97: no approval gate before publishing, mutable tags) is separate. A signature made by a
  compromised pipeline still verifies.
