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

Every release publishes `SHA256SUMS` and a signed build-provenance attestation for each archive.
Before running a downloaded binary, check both:

```bash
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify <archive> --repo IAmSkweetis/branch-tally
```
