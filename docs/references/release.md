# Releases

How a version of branch-tally gets from `main` to users.

> **Status:** GitHub Releases ship six prebuilt binaries with release notes, `SHA256SUMS` and build
> provenance attestations (#10). Not built yet, each a follow-up issue: publishing to crates.io and
> `cargo binstall`, Homebrew, npm, shell and PowerShell installers, and the GitHub Action's
> floating major tag (`v1`).

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
| release | `publish` | Downloads the six archives, writes `SHA256SUMS`, attests every archive, uploads them to the draft, then publishes it as **latest** | Fewer than six archives, or an upload or attestation error |

The order is deliberate. The tag exists before the build, so a failed build leaves a draft release
and a tag, and the next release still bumps from that tag. A draft is invisible and never "latest",
so **latest always has binaries**.

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
`scripts/smoke-test.sh` or `cliff.toml`. It builds the full matrix with the dummy version `v0.0.0`
and uploads nothing to any release.

## Verifying a download

Every archive has a line in `SHA256SUMS` and a GitHub build-provenance attestation.

```bash
sha256sum --check --ignore-missing SHA256SUMS          # shasum -a 256 -c on macOS
gh attestation verify branch-tally-v0.1.0-<target>.tar.gz --repo IAmSkweetis/branch-tally
```

The attestation proves the archive was built by this repository's `release.yml` from the tagged
commit.

## Related pages

- [dev_workflow.md](dev_workflow.md#commit-messages): the commit and PR-title rules versioning reads
- [agents.md](agents.md): who owns release configuration
