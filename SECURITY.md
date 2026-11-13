# Security Policy

branch-tally reads AI-agent transcripts, which contain prompts, code and secrets, and it runs in
CI with a token that can edit pull requests. We treat its security, and the privacy of what it
reads, as part of the product.

## Supported versions

branch-tally is before 1.0. Only the **latest release** receives security fixes. A release is cut
from every green push to `main`, so a fix ships as soon as it lands.

| Version | Supported |
| ------- | --------- |
| Latest release | Yes |
| Anything older | No: upgrade to the latest release |

## Reporting a vulnerability

**Do not open a public issue, pull request or discussion for a vulnerability.**

Report it privately through GitHub:
[**Report a vulnerability**](https://github.com/IAmSkweetis/branch-tally/security/advisories/new)
(the Security tab, then "Report a vulnerability"). This opens a private advisory that only you and
the maintainer can see.

Please include:

- the branch-tally version (`branch-tally --version`), OS, and how you installed it (release
  binary, GitHub Action, Claude Code plugin, or from source);
- what an attacker can do, and what they need first;
- the smallest reproduction you can manage.

**Never attach a real transcript, or any part of one.** It may hold your own secrets. Reduce the
reproduction to the fields branch-tally reads (usage, model, ids, timestamps, branch, cwd and
attribution) with placeholder values, or scrub it as described in
[testing.md](docs/references/testing.md#fixtures). If a vulnerability can only be shown with real
content, describe it and we'll work out a safe way to share it.

## What to expect

- **Acknowledgement within 7 days.** branch-tally has one maintainer, so everything after that is
  best effort, and you'll be kept up to date in the advisory.
- Once confirmed, we fix it on a private branch, release the fix, and publish the advisory, with a
  CVE where one applies. You'll be credited unless you'd rather not be.
- Please give us a reasonable chance to release a fix before disclosing publicly. We'll agree a
  date with you in the advisory.

## Scope

In scope:

- **Transcript content leaking.** Any path by which prompt, response or code text from a
  transcript reaches PR output, the ledger, logs, error messages or any other output. branch-tally
  is designed never to emit content, so any leak is a vulnerability, not just a bug.
- **Injection through transcript-derived values.** Branch names, paths, model names and other
  strings read from transcripts or git that can break out of the Markdown report, alter a PR body
  beyond the token-report block, or reach a shell or `gh` command.
- **Reading outside its scope.** branch-tally reading files other than the harness transcripts and
  repository state it needs.
- **Supply chain and distribution.** Release binaries, checksums and attestations, the GitHub
  Action, the Claude Code plugin, and installers and package-registry releases.
- **This repository's CI.** Workflow weaknesses in `.github/workflows` that could compromise
  releases or secrets.

Out of scope:

- Vulnerabilities in Claude Code or other agent harnesses themselves. Report those to their
  vendors.
- Transcripts a user has already exposed, for example by committing them.
- Cost figures that are wrong without a security impact. Those are bugs: please open an issue.

## Verifying a release

Every release publishes `SHA256SUMS`, a minisign signature of it (`SHA256SUMS.minisig`) and a
Sigstore build-provenance attestation (`provenance.sigstore.json`) whose subjects are the archives,
the installers, `SHA256SUMS` and `SHA256SUMS.minisig`.

`SHA256SUMS` is an **integrity check, not proof of origin**: it catches a corrupt or truncated
download, but anyone who can replace an archive can replace the checksum file next to it. Proof of
origin comes from the signature and the attestation. Before running a downloaded binary, check
them:

```bash
# Get minisign.pub from .github/minisign.pub in this repository, not from the release page.
minisign -Vm SHA256SUMS -p minisign.pub
sha256sum --check --ignore-missing SHA256SUMS      # shasum -a 256 -c on macOS
# One attestation covers every asset: an archive, an installer, SHA256SUMS or SHA256SUMS.minisig.
gh attestation verify <archive> \
  --repo IAmSkweetis/branch-tally \
  --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
  --source-ref refs/heads/main
gh attestation verify SHA256SUMS \
  --repo IAmSkweetis/branch-tally \
  --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
  --source-ref refs/heads/main
```

`--repo` alone accepts an attestation from any workflow in the repository, so the commands pin the
signer workflow and the ref too. `minisign -V` prints a trusted comment,
`branch-tally <tag> SHA256SUMS`. Check that the tag is the release you downloaded: it stops a
signature from another release being replayed. The signing key exists only as a GitHub Actions
secret used by the release job. Details are in
[release.md](docs/references/release.md#verifying-a-download).
