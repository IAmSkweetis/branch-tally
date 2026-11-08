# Releases

How a version of branch-tally gets from `main` to users.

> **Status:** the release machinery is #10. The principles below are decided. The tooling marked
> *proposed* is a recommendation that #10 confirms or replaces. Update this page when it lands.

## Principles (decided)

- **Releases are cut from `main`, and only from `main`.** Trunk-based development has no release
  branch to stabilise on. `main` is already stable, because every PR into it passed the gates.
- **A release is a tag**, `vMAJOR.MINOR.PATCH`, on a commit already on `main`. Every published
  artifact is built by CI from that tag: binaries, crates.io, Homebrew, npm and the Action's major
  tag. Nothing is built or published from a laptop. `cargo publish` is denied in
  `.claude/settings.json` for that reason.
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
- **Leaving 0.x is a deliberate act**, never a side effect of one breaking commit.
- **Release notes are generated** from the same history, grouped by commit type. The GitHub release
  page is the changelog for users.
- **Fixes go to `main` first.** A `hotfix/` branch is cut from `main` like any other. If an older
  release line ever needs a patch, the fix is cherry-picked from `main` onto a `release/vX.Y`
  branch cut from that tag. That branch exists only to publish the patch and is never merged back.

## Proposed tooling (#10)

Unlike a project whose version lives only in a git tag, a crate's version must be **written in
`Cargo.toml`** for crates.io to accept it. That favours a **release-PR** flow:

1. [release-plz](https://release-plz.dev/) watches `main`. When releasable commits land, it opens or
   updates a PR that bumps `Cargo.toml` versions and the changelog, using the table above.
2. Merging that PR, which is still a squash merge through the normal gates, tags the commit.
3. The tag triggers [cargo-dist](https://opensource.axo.dev/cargo-dist/). It builds binaries for
   Linux, macOS (x64 and arm64) and Windows, writes shell and PowerShell installers, and publishes
   the GitHub release, the Homebrew formula and the npm wrapper.
4. The same tag moves the Action's floating major tag (`v1`) so `uses: …@v1` picks it up.

Batching releases through a release PR (rather than releasing every merge) keeps users from seeing
a patch release for every docs or CI change. The trunk is still always releasable; we just choose
when to cut.

Open questions for #10: whether `branch-tally-core` is published to crates.io or kept
`publish = false`; artifact signing; and the npm package name.

## Related pages

- [dev_workflow.md](dev_workflow.md#commit-messages): the commit and PR-title rules versioning reads
- [agents.md](agents.md): who owns release configuration
