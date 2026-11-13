# Releases

How a version of branch-tally gets from `main` to users.

> **Status:** GitHub Releases ship six prebuilt binaries with release notes, a minisign-signed
> `SHA256SUMS` and build provenance attestations (#10, #76), plus one-line shell and PowerShell
> installers (#46), proven on clean Linux, macOS and Windows runners after every release. Not built
> yet, each a follow-up issue: publishing to crates.io and `cargo binstall`, Homebrew, npm, and the
> GitHub Action's floating major tag (`v1`).

## Principles (decided)

- **Releases are cut from `main`, and only from `main`.** Trunk-based development has no release
  branch to stabilise on. `main` is already stable, because every PR into it passed the gates.
- **A release is a tag**, `vMAJOR.MINOR.PATCH`, on a commit already on `main`. Every published
  artifact is built by CI from that tag. Nothing is built or published from a laptop.
  `cargo publish` is denied in `.claude/settings.json` for that reason.
- **Every merge to `main` releases.** There is no release PR and no batching, so a docs-only or
  tooling-only merge is a patch release. The trunk is always releasable, and the releases page
  always reflects it.
- **The version comes from the commit history.** Squash-merged PR titles are Conventional Commits,
  so the next version follows mechanically from what merged since the last tag:

  | Since the last tag | Before 1.0 | From 1.0 |
  | ------------------ | ---------- | -------- |
  | any `!` / `BREAKING CHANGE` | minor | major |
  | any `feat` | minor | minor |
  | only `fix`, `perf`, and others | patch | patch |

  "Breaking" means the public contract: CLI flags and exit codes, the Markdown and JSON output
  shape, the ledger format, and the Action's inputs and outputs. It does not include the Rust API
  of `branch-tally-core` while that crate is unpublished.
- **Leaving 0.x is a deliberate act**, never a side effect of one breaking commit. Dispatch the
  workflow with `-f version=v1.0.0`.
- **Release notes are generated** from the same history, grouped by commit type. The GitHub release
  page is the changelog for users. There is no `CHANGELOG.md`.
- **Fixes go to `main` first.** A `hotfix/` branch is cut from `main` like any other. If an older
  release line ever needs a patch, the fix is cherry-picked from `main` onto a `release/vX.Y`
  branch cut from that tag. That branch exists only to publish the patch and is never merged back.

## How a release starts

Two workflows share the work:

- **`release.yml`** decides the version, tags, drafts the GitHub release and publishes it.
- **`build.yml`** is reusable (`workflow_call`). It builds, smoke-tests and packages the six
  targets, and `release.yml` calls it.

A release starts in one of two ways.

1. **Automatically**, when the `CI` workflow completes **successfully for a push to `main`**
   (`on: workflow_run`). The jobs run only if the CI run succeeded, was a `push`, and came from this
   repository rather than a fork's pull request (a fork PR can have `head_branch` `main`). They act
   on that run's `head_sha`, never on `github.sha`. zizmor flags `workflow_run` as a dangerous
   trigger; the inline `# zizmor: ignore[dangerous-triggers]` comment on it explains those guards,
   and the audit stays on everywhere else.
2. **By hand**, with `workflow_dispatch` and an optional `version` input:

   ```bash
   gh workflow run release.yml                      # version chosen by git-cliff
   gh workflow run release.yml -f version=v1.0.0    # name the version
   ```

   Dispatch refuses unless it runs from `main`, and unless the `CI` workflow has a successful
   `push` run for that exact commit. It fails when there are no commits since the last tag.

Every release builds. There is no label or input to turn that off.

## The jobs, in order

| Workflow | Job | Does | Fails when |
| -------- | --- | ---- | ---------- |
| release | `version` | Checks the commit is releasable, works out the version, writes the release notes (also to the run summary) | A dispatch is off `main` or the commit has no green CI push run, no commits since the last tag (dispatch), a malformed version, or the tag exists |
| release | `release` | Creates the tag on the exact commit through the API, then a **draft** GitHub release with the notes | The tag exists on a different commit |
| build | `build` (x6) | Installs the pinned toolchain and the target, builds with `--locked`, smoke-tests the binary, packages it | Compilation fails, `--version` is not `branch-tally X.Y.Z`, `report` fails on the fixtures, or packaging fails |
| release | `publish` | Downloads the six archives, fetches `install.sh` and `install.ps1` from the tagged commit, writes `SHA256SUMS`, signs it with minisign (`SHA256SUMS.minisig`) and verifies the signature against the committed public key, attests the archives, installers, `SHA256SUMS` and `SHA256SUMS.minisig` (bundle shipped as `provenance.sigstore.json`), uploads everything to the draft, then publishes it as **latest** | Fewer than six archives, an installer missing at the tagged commit, a missing signing secret, the placeholder public key, a signature that does not verify, or an upload or attestation error |
| install-check | `resolve`, `check` (x6), `signature` | After a successful `Release` run, resolves the released tag and runs the real one-liners on a clean runner of each target, including with `--require-attestation` through `gh` and through `cosign`; `signature` verifies `SHA256SUMS.minisig` with minisign ([Installing](#installing)) | The installed binary's `--version` is not the released tag, the default directory was not used, the latest release has no installer, a just-published release lacks `SHA256SUMS`, `SHA256SUMS.minisig` or `provenance.sigstore.json`, provenance does not verify by either route, or the signature does not verify against `.github/minisign.pub` at the tag with trusted comment `branch-tally <tag> SHA256SUMS` |

The order is deliberate. The tag exists before the build, so a failed build leaves a draft release
and a tag, and the next release still bumps from that tag. A draft is invisible and never "latest",
so **latest always has binaries**.

`install-check.yml` is a separate workflow started by `workflow_run` when `Release` completes (the
same trigger and guards as `release.yml`: this repository only, successful runs only, no checkout,
no artifacts read). It also takes a `workflow_dispatch` with an optional `version`, to re-check a
release by hand. If the `Release` run succeeded without publishing (a newer run released past its
commit), it says so in a notice and checks whatever is latest; a latest release that predates the
installers is skipped with a notice, never failed. The same holds for the provenance steps: a manual
check of a tag from before signing (no `SHA256SUMS.minisig` or `provenance.sigstore.json`) skips
them with a notice, while a release the `Release` run just published without those assets is an
error.

### Superseded runs

Releases run one at a time (`concurrency: group: release`, never cancelling). While one runs, a
newer CI-passed commit can queue behind it, and a still-newer one replaces the queued run. That is
fine: the next release includes the replaced commit. When a run starts and its commit is already
contained in the latest `v*` tag, it exits with a notice instead of failing, since a newer run
released past it.

## Versioning

The version lives **only in the git tag** (`v0.4.0`). Nothing in the repository is bumped, no
release commit lands on `main`, and `Cargo.toml` stays at `0.0.0`. git-cliff picks it from the
Conventional Commit history since the last `v*` tag, using `cliff.toml`:

- a `feat` bumps the minor;
- a `!` or `BREAKING CHANGE` bumps the minor while the version is `0.x`, and the major from `1.0`
  on;
- anything else bumps the patch.

`features_always_bump_minor` keeps a `feat` a minor bump while the version is `0.x`.
`breaking_always_bump_major = false` keeps a breaking change from leaving `0.x` on its own. The
commits it reads are the squash-merged PR titles, which is one more reason the PR title has to be
a valid Conventional Commit (see [dev_workflow.md](dev_workflow.md#commit-messages)).

The version is validated as `vMAJOR.MINOR.PATCH` and refused if the tag already exists. The build
passes it to the compiler as `BRANCH_TALLY_VERSION` (without the `v`), and `branch-tally --version`
prints `branch-tally X.Y.Z`. A local build without the variable prints `branch-tally 0.0.0-dev`.

git-cliff is pinned in the `justfile` (`cliff_version`); the workflow reads it back with
`just --evaluate cliff_version`. Preview locally, writing nothing:

```bash
just next-version      # the version the next release would get
just release-notes     # the notes for the unreleased commits
```

Both need git-cliff installed at the pin and the full git history.

## Release notes

git-cliff writes the notes for the commits since the last tag, grouped by commit type, to the draft
release and the run summary of the `version` job.

## Re-runs and their limits

Use **Re-run failed jobs** on the `release.yml` run to retry a failed build or publish. It reuses
the same tag and builds the same commit. Don't dispatch a new release for a failed one: that would
mint a new version.

- A re-run uses the workflow files **from the original commit**, so it cannot pick up a fix to
  `build.yml` or `release.yml`. When the fix is a workflow change, land it on `main`: the next
  release builds with the fixed workflow, and the failed one stays a draft with its tag.
- The `release` job is safe to re-run: an existing tag on the same commit and an existing release
  are left alone.
- Archives are kept as run artifacts for 14 days. A re-run of only the `publish` job after that
  fails for lack of archives; re-run the whole workflow instead.
- A failed draft is never deleted for you. Delete it by hand once superseded.

## Targets

Each target is built on a native runner, so each binary is smoke-tested where it was built.

The two musl targets need a musl C compiler, because `ring` compiles C and assembly. Ubuntu has
no `<arch>-linux-musl-gcc`, so `build.yml` installs `musl-tools` and sets `CC_<target>=musl-gcc`.
That works only because each musl target builds on a runner of its own architecture.

| Target | Runner | Archive |
| ------ | ------ | ------- |
| `x86_64-unknown-linux-musl` | `ubuntu-24.04` | `.tar.gz` |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` | `.tar.gz` |
| `x86_64-apple-darwin` | `macos-15-intel` | `.tar.gz` |
| `aarch64-apple-darwin` | `macos-15` | `.tar.gz` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `.zip` |
| `aarch64-pc-windows-msvc` | `windows-11-arm` | `.zip` |

An archive is `branch-tally-<vX.Y.Z>-<target>.tar.gz` (or `.zip`) holding one directory with the
binary, `LICENSE` and `README.md`. `scripts/package.sh` builds it (`just package`), and
`scripts/smoke-test.sh` runs the checks: `--version` must equal `branch-tally X.Y.Z` exactly, and
`report` must succeed against the committed Claude Code fixtures in a throwaway repository and
home directory, so it never reads the runner's transcripts.

### Proving the build before a release

`release.yml` runs only from `main`, so it cannot be tried on a branch. `build.yml` therefore also
runs on a **pull request** that changes `build.yml`, `release.yml`, `scripts/package.sh`,
`scripts/smoke-test.sh`, `scripts/sign-checksums.sh`, `scripts/ci-install-minisign.sh`,
`.github/minisign.pub`, `cliff.toml`, `Cargo.lock` or `rust-toolchain.toml`. It builds the full
matrix with the dummy version `v0.0.0` and uploads nothing to any release. A `signing proof` job
runs only on pull requests: it installs minisign, signs a dummy `SHA256SUMS` with a throwaway key
generated in the job and verifies it, and checks the script refuses a placeholder public key. It
needs no secret and cannot touch the real key. The lockfile is on the list because a dependency can
break one target only: `ring` (rustls's crypto, added in #53) compiles C, and nothing outside the
release build compiles for musl.

## Installing

Two installers, `install.sh` and `install.ps1`, live at the repository root and are attached to
every release (checksummed in `SHA256SUMS` and attested, like the archives). The one-liners:

```bash
curl -LsSf https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex"
```

| Flag (`install.sh`) | Parameter (`install.ps1`) | Environment variable | Meaning |
| ------------------- | ------------------------- | -------------------- | ------- |
| `--version <vX.Y.Z>` | `-Version` | `BRANCH_TALLY_INSTALL_VERSION` | Install that release instead of the latest |
| `--to <dir>` | `-To` | `BRANCH_TALLY_INSTALL_DIR` | Install directory |
| `--require-attestation` | `-RequireAttestation` | `BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION` | Fail unless provenance was verified (`1`, `true` or `yes` for the variable) |
| none | none | `BRANCH_TALLY_INSTALL_BASE_URL` | Releases URL, `https://` or `file://` only, default `https://github.com/IAmSkweetis/branch-tally/releases`. The effective source is printed when it is set. The tests point it at a `file://` fake release |
| none | none | `BRANCH_TALLY_INSTALL_API_URL` | Where "latest" is looked up, `https://` or `file://` only, default `https://api.github.com`. Printed as `Using release API: <url>` when set. The same trust as the mirror variable. The tests point it at a `file://` fake API |
| none | none | `GH_TOKEN`, else `GITHUB_TOKEN` | Sent as a bearer token to the "latest" lookup only, to avoid rate limits. Never printed and never on a command line: curl reads it from a private header file, GNU wget 1.17+ from a private `--config` file, and any other wget (BusyBox) goes unauthenticated with a notice |

With nothing set, the binary goes to `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows). The
installer **never edits PATH**: when the directory is not on it, it prints a hint. On success it
prints the installed path and the binary's `--version`. With options, use the flags
(`curl ... | sh -s -- --version v0.1.0 --to ~/bin`). `irm | iex` cannot pass parameters, so on
Windows use the environment variables.

**Which release is "latest".** With no version pinned, both installers read `tag_name` from
`<api>/repos/IAmSkweetis/branch-tally/releases/latest` (validated as `vMAJOR.MINOR.PATCH`), **even
when a mirror is set**: a signature proves a release is genuine, not that it is the newest, so a
mirror serving an older signed release as "latest" would otherwise roll users back. The JSON is
read without `jq` (POSIX `sh`: a line-oriented `sed` on `"tag_name"`; PowerShell:
`ConvertFrom-Json`), then validated strictly. When the API is unreachable, rate limited or
answers with anything unusable, the installers fall back to the mirror's
`latest/download/SHA256SUMS` and print a loud stderr warning that "latest" came from the mirror and
was **not confirmed by GitHub**, so an older release could be served, with the `--version` route.
The fallback is allowed under `--require-attestation` too, because the archive is still verified.
Rollback protection is therefore partial: it does not hold against an attacker who can intercept
`api.github.com`, nor in the fallback ([decision 0004](../decisions/0004-release-provenance.md)).

**Verification.** The installer downloads the archive for the platform and checks it against the
release's `SHA256SUMS`, and stops on a mismatch. That is an **integrity check**: it catches a
corrupt or truncated download, and proves nothing about origin, because the checksum comes from the
same place as the archive. Only the build-provenance attestation proves origin
([decision 0004](../decisions/0004-release-provenance.md)). After the checksum check the installer
verifies it with the first tool that is available:

1. `gh`, when on PATH and logged in (`gh auth status` succeeds): `gh attestation verify <archive>
   --repo IAmSkweetis/branch-tally --signer-workflow
   IAmSkweetis/branch-tally/.github/workflows/release.yml --source-ref refs/heads/main
   --cert-oidc-issuer https://token.actions.githubusercontent.com`.
2. Otherwise `cosign`, when on PATH: `cosign verify-blob-attestation` of the archive against the
   release asset `provenance.sigstore.json`, with an exact `--certificate-identity
   https://github.com/IAmSkweetis/branch-tally/.github/workflows/release.yml@refs/heads/main`
   (no regexp) and `--certificate-oidc-issuer https://token.actions.githubusercontent.com`. A
   release with no bundle counts as "could not verify", not as a pass.
3. Otherwise it warns on stderr that provenance was **not** verified, prints the full `gh
   attestation verify` command above (with the same pinned flags) to run, and installs.

The signer is pinned to `release.yml` on `main`, not merely to the repository: `--repo` alone
accepts an attestation from any workflow in the repository that can write one, and
`release.yml` also starts from `workflow_dispatch`, which can run on any branch. A release is
the `workflow_run` on `main`, so any other ref is rejected.

A verification that runs and fails is a hard failure: nothing is installed, and `cosign` is not
tried after a failing `gh`. `--require-attestation` (`-RequireAttestation`, or
`BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION`) turns case 3, and a missing bundle, into a failure too.
The dependency floor without `gh` or `cosign` is unchanged: `curl` or `wget`, `sha256sum` or
`shasum`, and `tar` on Linux and macOS, and built-in PowerShell on Windows (Windows PowerShell 5.1
and PowerShell 7 both work). A `curl | sh` install stays dependency-free and says so when it could
not verify.

**Transport hardening (F-06).**

- `install.sh` runs curl with `--proto '=https' --proto-redir '=https' --tlsv1.2`, and wget with
  `--https-only` when that wget has it (BusyBox's does not; the URL scheme check still applies).
  `file://` mirrors are plain copies, the seam the tests use. `install.ps1` pins TLS 1.2+ and
  follows redirects by hand (`-MaximumRedirection 0`, at most 5 hops), refusing any hop that is not
  `https://`. The documented one-liner runs Windows PowerShell 5.1, whose `Invoke-WebRequest` would
  otherwise follow an HTTPS-to-HTTP redirect (#98). The scheme checks (`file://`, `https://`) are
  case-insensitive in both installers. The PowerShell version check uses `\z`, not `$`, which in
  .NET also matches before a trailing newline; the `sh` check is a `case` pattern and already
  rejects one.
- `BRANCH_TALLY_INSTALL_BASE_URL` and `BRANCH_TALLY_INSTALL_API_URL` must be `https://` or
  `file://`, the same rule as `branch-tally update`. Plain `http://` is refused. When either is set,
  the installer prints `Using release mirror: <url>` or `Using release API: <url>`.
- `install.sh` is a set of functions plus `main "$@"` on its last line, so a download cut off
  partway is a syntax error and runs nothing.
- The tag read from the remote `SHA256SUMS` under `latest` must be `vMAJOR.MINOR.PATCH`, the check
  `--version` already had.

Still open for the installers: a mirror set by the environment is honoured without a prompt (the
`update --yes` half of F-06 is separate).

### Updating in place

`branch-tally update` (alias `upgrade`) is the in-place counterpart of the installers. The decision
logic lives in core's `update` module; the CLI supplies the network, archive and replacement. It
fetches the same archives and resolves the tag from the latest release or `--version`. It is strict
about origin ([0004](../decisions/0004-release-provenance.md)). Before trusting any checksum it
downloads `SHA256SUMS` and `SHA256SUMS.minisig` for that tag and verifies the signature (proof of
origin) over the exact `SHA256SUMS` bytes against the release public key, refusing legacy
(non-prehashed) signatures. The signed trusted comment must be exactly
`branch-tally <tag> SHA256SUMS` for the tag being installed. Only then does it fetch the archive and
check it against `SHA256SUMS` (an integrity check) and install. Each failure is its own typed
`UpdateError` and installs nothing: a missing signature (a release from before signing, also through
`--version`; the message says the release is not signed and points at `gh attestation verify`), a
malformed or invalid signature, a signature by another key, a legacy signature, and a trusted
comment for another tag or file. A mismatch or an archive missing from `SHA256SUMS` is likewise a
hard failure. The key is an argument to `update::run_verified` (`ReleaseKey`); core reads no
environment variable or file for it, so the binary's compiled-in key is the only one. A mirror is
fine, since trust comes from the signature. A dev build (`0.0.0-dev`) and an unwritable binary
directory are refused, with the installer one-liner in the message. `--check` downloads nothing;
`--version` may downgrade; `--force` reinstalls the current version; `--yes` skips the prompt.

Without `--version`, `update` never installs a release older than the running binary, `--force`
included. Core returns `Outcome::LatestOlder` and the message says the latest release (vX) is older
than the running version (vY), nothing was installed, and `--version` is the way back. It is not
"up to date" (that would hide the mismatch) and not an error (exit 0, nothing went wrong), and
`--check` reports it the same way instead of offering an update. If "latest" can't be resolved
(`ReleaseClient::latest_tag` fails), the error is `UpdateError::LatestUnresolved`: it says the
latest release could not be resolved and to pin one with `--version vX.Y.Z`, with URLs scrubbed.
`latest_tag` must resolve from the GitHub API, independent of the mirror, and never fall back to it.
Core's tests use throwaway keys and signatures in
`crates/branch-tally-core/tests/data/update_signing/`.

`BRANCH_TALLY_INSTALL_BASE_URL` (the installers' mirror variable) is honoured, but only for
`https://` and `file://`; any other scheme, plain `http://` included, is a one-line error before a
network call. Downloads and the extracted binary are capped at 256 MiB, and exceeding the cap is an
error, never a truncated install. The new binary is staged beside the old one with an exclusive
create (a planted symlink is not followed), flushed to disk, given the old binary's permission bits
(`0o755` if unreadable) and renamed into place. On Windows the running exe is first renamed to
`branch-tally.exe.old`; the next update that actually installs removes it, and says so if it cannot
(for example because it is still running). If moving the new binary in and restoring the old one
both fail, the error states that the binary now lives at `<path>.old` and to rename it back.

### Proving an install works

Two layers. `installers.yml` runs on a pull request or push to `main` that changes `install.sh`,
`install.ps1`, `tests/install/**`, the `scripts/lint-install.*` and `scripts/test-install.sh`
helpers or the workflow itself: ShellCheck on the shell files, PSScriptAnalyzer (pinned, warnings
fail) on the PowerShell files, and the two offline test scripts (`sh tests/install/install_test.sh`,
`pwsh -NoProfile -File tests/install/install_test.ps1`) on Ubuntu, macOS and Windows. On a Windows
host the sh suite is skipped with a message, because `install.sh` does not target Windows; the pwsh
suite still runs, and a missing `pwsh` there still fails when `CI` is set. It is
path-filtered like `workflow-audit.yml` and none of its jobs are required checks, so a pull request
that touches none of those paths is not left waiting. Locally: `just lint-install` and
`just test-install`, which print a skip when `shellcheck` or `pwsh` is absent and fail when `CI` is
set.

Then `install-check.yml` proves the published thing, on six runners (`ubuntu-24.04`,
`ubuntu-24.04-arm`, `macos-15`, `macos-15-intel`, `windows-latest`, `windows-11-arm`), none of
which checks out the repository. Each asserts the default directory is empty, runs the real
one-liner, checks the binary landed in the default directory and that `--version` equals the tag
just released, then repeats with `--version` and a custom directory. Windows does the default
install twice, under Windows PowerShell 5.1 and under PowerShell 7.

The same runners then check provenance for real, because without a token or `cosign` the installers
only warn that it was not verified and pass. Each runs the pinned one-liner from the release under
test with `--require-attestation` (on Windows `BRANCH_TALLY_INSTALL_REQUIRE_ATTESTATION=1`, since
`irm | iex` takes no parameters) twice, and asserts the output names the route taken:

- **gh**: with the job's token (`GH_TOKEN`, and `attestations: read` on the job). The output must
  say `Provenance verified with gh`.
- **cosign**: installed by `sigstore/cosign-installer` pinned by SHA, with `gh` made unusable
  (no token, an empty `GH_CONFIG_DIR`, and a check that `gh auth status` fails), so the installer's
  "gh, logged in" test fails and it takes the cosign route. The output must say
  `Provenance verified with cosign`. `gh` stays on `PATH` because on a runner it lives in a
  directory shared with system tools; what the installer tests is a logged-in `gh`.

One more job, `signature`, downloads `SHA256SUMS` and `SHA256SUMS.minisig` from the release,
installs minisign with `scripts/ci-install-minisign.sh`, and verifies them against
`.github/minisign.pub`, requiring the trusted comment to be exactly `branch-tally <tag> SHA256SUMS`.
The key and the installer script are fetched from the **tagged commit** through the API, since the
workflow checks nothing out. A failure here comes after the release is public: fix forward with a
new release.

## Verifying a download

What a release contains and what proves what
([decision 0004](../decisions/0004-release-provenance.md)):

| Asset | What it is | Proves |
| ----- | ---------- | ------ |
| archives, `install.sh`, `install.ps1` | The artifacts | Nothing alone |
| `SHA256SUMS` | sha256 of every archive and installer | **Integrity only**: the download is not corrupt. Whoever can replace an archive can replace this file beside it |
| `SHA256SUMS.minisig` | minisign (Ed25519) signature of `SHA256SUMS`, trusted comment exactly `branch-tally <tag> SHA256SUMS` | Origin: the maintainer's key signed this checksum file for this tag. A signature for one tag can't be replayed as another |
| `provenance.sigstore.json` | Sigstore attestation bundle whose subjects are the archives, installers, `SHA256SUMS` and `SHA256SUMS.minisig` | Origin: this repository's `release.yml` built them from the tagged commit |

The `publish` job signs through `scripts/sign-checksums.sh`. Setup the maintainer owns:

- **Secrets** (repository Actions secrets, read only by the `Sign SHA256SUMS` step):
  `MINISIGN_SECRET_KEY` (the whole contents of the minisign secret key file) and
  `MINISIGN_PASSWORD` (its password; set it to an empty value for a key made with
  `minisign -G -W`). If either is absent the job fails and nothing is published.
- **Public key**: `.github/minisign.pub`, the maintainer's (the `.pub` file `minisign -G` writes).
  The job fails on a placeholder, and `just test-scripts` fails if the committed file stops being a
  real key. The same key is compiled into the binary for `branch-tally update`.
- **minisign** is installed by `scripts/ci-install-minisign.sh` from the upstream release archive,
  pinned by version and sha256 like gitleaks. Bump both together, checking the new archive against
  upstream's `.minisig` and key.
- The job verifies its own signature against the committed public key and checks the trusted
  comment before it uploads anything. A secret key that doesn't match the committed public key
  fails there.
- Losing or leaking the key means a new key pair and a new binary that embeds it: rotation is a
  release.

To verify a download by hand:

```bash
minisign -Vm SHA256SUMS -p minisign.pub      # the key from .github/minisign.pub; checks SHA256SUMS.minisig
sha256sum --check --ignore-missing SHA256SUMS   # shasum -a 256 -c on macOS
# Pin the signer workflow and ref: --repo alone accepts any workflow in the repository.
# The same command works for any asset: an archive, an installer, SHA256SUMS or SHA256SUMS.minisig.
gh attestation verify branch-tally-v0.1.0-<target>.tar.gz \
  --repo IAmSkweetis/branch-tally \
  --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
  --source-ref refs/heads/main
gh attestation verify SHA256SUMS \
  --repo IAmSkweetis/branch-tally \
  --signer-workflow IAmSkweetis/branch-tally/.github/workflows/release.yml \
  --source-ref refs/heads/main
```

The first command's output includes `Trusted comment: branch-tally <tag> SHA256SUMS`: check the tag
is the one you downloaded. The checksum then ties each archive to the signed file. The attestation
independently proves the archive was built by this repository's `release.yml` from the tagged
commit. F-05 (#97, no approval gate and mutable tags) is separate: a signature made by a
compromised pipeline still verifies.

## Related pages

- [dev_workflow.md](dev_workflow.md#commit-messages): the commit and PR-title rules versioning reads
- [agents.md](agents.md): who owns release configuration
