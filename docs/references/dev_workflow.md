# Development workflow

How a change gets from an idea to `main`: where it is written, what it is named, and what has to be
green before it lands.

## Trunk-based development

`main` is the trunk. Everything integrates there, often, and it is kept **always green and always
releasable**. There is no `develop` branch and no long-lived release branch. Releases are tags cut
from `main` ([release.md](release.md)).

What that asks of a change:

- **Branches are short-lived.** Aim for hours to a couple of days. A branch that lives for weeks
  turns every merge into a merge project. If the work is big, split it into issues that each land
  on their own.
- **Each branch maps to one roadmap issue** where there is one. The issue says what "done" means,
  and the PR closes it.
- **Land incomplete work dark, not late.** It is fine to merge a module no command exposes yet, or a
  subcommand marked `#[command(hide = true)]`. It is not fine to merge something that breaks `main`
  or changes released behaviour half-way. "Dark" means unreachable from the shipped surface, not
  untested: Governance I still applies in full.
- **`main` moves under you.** Rebase or merge `main` into your branch before opening the PR, and
  again if CI on `main` has changed since. Never force-push a branch someone else has pulled, and
  never force-push `main`.

## The loop

1. **Branch** from an up-to-date `main`, in a git worktree if other work is running in parallel.
2. **Write the test first.** Governance I: red, then green. See [testing.md](testing.md).
3. **Commit** in Conventional Commits format, as often as is useful.
4. **Push** the branch and open a **pull request** (`/create-gh-pr`).
5. **CI gates the PR** with the same checks the hooks run, plus the cross-platform test matrix.
6. **Squash-merge to `main`** once CI is green. It is the only merge method this repository allows.
7. **Clean up** the worktrees and branches the work left behind, after the merge and not before
   (`/worktree-cleanup`).

Never commit directly to `main`, never force-push, and never merge work without the PR.

### Squash merge rewrites history, so plan around it

A squash merge collapses every commit on the branch into one new commit on `main`, whose parent is
whatever `main` was before. **None of the branch's own commits become ancestors of `main`.** So
`git branch --merged`, `git merge-base --is-ancestor` and "`git log main..<branch>` is empty" stop
meaning "did this land?" as soon as the PR merges. To check whether work landed, compare
**content** (`git diff origin/main <branch>`), not ancestry. `worktree-cleanup` does exactly that.

### Repository settings this depends on

These are set in GitHub, not in the repository. Apply them before the first PR merges:

| Setting | Value | Why |
| ------- | ----- | --- |
| Allowed merge methods | squash only | One commit per PR on `main`. The PR title is the commit message the release tooling reads. |
| Default squash message | PR title and description | So the Conventional Commit written in the title is what lands. |
| Branch protection on `main` | require a PR, require CI green, block force-pushes and deletion | The hooks stop accidents locally; this stops them for everyone. |
| Automatically delete head branches | off | `worktree-cleanup` deletes branches after checking their content landed. |
| Required status checks on `main` | `lint`, `lint-md`, `scripts`, `test (ubuntu-latest)`, `test (macos-latest)`, `test (windows-latest)`, `msrv`, `deny`, `secret-scan`, `pr-title`, `branch-name`, `commit-messages`, `dependency-review` | The CI jobs above. Don't require `zizmor`: it only runs when `.github/` changes, so on other PRs it would never report. |
| Dependency graph, Dependabot alerts, Dependabot security updates | on | Feeds `dependency-review` and the Dependabot PRs. |

## Branch names

Every branch follows [Conventional Branch](https://conventionalbranch.org/):

```text
<type>/<description>
```

The description uses lowercase `a-z`, `0-9` and hyphens only, with no underscores or spaces. A
hyphen never starts or ends it, or appears twice in a row. `main` is the trunk and takes no prefix.

Conventional Branch has fewer types than Conventional Commits, so pick the branch type from the
commit type you expect to write:

| Commit type you'll write | Branch prefix |
| ------------------------ | ------------- |
| `feat` | `feat/` |
| `fix` | `fix/` |
| `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore` | `chore/` |
| an urgent fix to an already-released version | `hotfix/` |
| preparing a release, when release tooling needs a branch | `release/` |

`feature/` and `bugfix/` are equally valid spellings of the first two. Prefer `feat/` and `fix/` so
a branch and its commits read the same way. Dots are allowed in a `release/` description, for
versions: `release/v0.3.0`.

Include the issue number when there is one:

```text
feat/issue-4-report-command
fix/issue-6-worktree-subagent-rollup
chore/issue-2-transcript-fixtures
```

Name a branch for the **change**, not for the person or agent making it.

## Commit messages

Every commit follows [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

| Type | For |
| ---- | --- |
| `feat` | a new user-visible capability |
| `fix` | a bug fix |
| `docs` | documentation only |
| `style` | formatting that does not change behaviour |
| `refactor` | a change that neither fixes a bug nor adds a feature |
| `perf` | a performance improvement |
| `test` | adding or correcting tests |
| `build` | the build system, Cargo manifests, dependencies, release packaging |
| `ci` | CI configuration and hooks |
| `chore` | anything else |

Scopes name the area: `core`, `cli`, `attribution`, `sources`, `pricing`, `ledger`, `render`,
`plugin`, `action`, `release`, `agents`. Use one when it helps a reader of the changelog.

A breaking change is flagged with `!` after the type or scope, or with a `BREAKING CHANGE:` footer.
For this project "breaking" means the CLI's flags or output contract, the ledger format, the JSON
schema, or the Action's inputs. Changing an internal Rust API is not breaking:

```text
feat(ledger)!: key records by harness and request id
```

Write the body to explain **why**, not what. The diff already says what.

The **PR title** must also be a valid Conventional Commit. Merges are squashed, so the title is the
message `main` actually carries, and the release tooling reads it to choose the next version.

## Commands

The `justfile` at the repository root is the entrypoint for every script, gate and build. `just` on
its own lists the recipes.

| Recipe | Runs | Raw equivalent |
| ------ | ---- | -------------- |
| `just ci` | everything below except `fmt`, `fmt-md`, `audit-workflows` and `docs-serve`: every gate CI runs, including `docs` | — |
| `just test [filter]` | nextest over the workspace (a run that matches no tests fails), then doctests | `cargo nextest run --workspace --no-tests=fail [filter]` then `cargo test --workspace --doc [filter]` |
| `just lint` | `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` | same |
| `just fmt` | `cargo fmt --all` | same |
| `just lint-md [paths…]` | `rumdl check`, pinned | `uvx rumdl@<pin> check .` |
| `just fmt-md [paths…]` | `rumdl fmt`, which rewraps long paragraphs | `uvx rumdl@<pin> fmt .` |
| `just secret-scan [args…]` | `gitleaks git` over the full history of every local ref, redacted. CI passes `--log-opts=HEAD` | `gitleaks git --redact .` |
| `just audit-workflows [--offline]` | `zizmor` over `.github/`, pinned. Online audits run when `GH_TOKEN` is set. | `uvx zizmor@<pin> .github/` |
| `just test-scripts` | proves `scripts/check-commit-subject.sh`, `scripts/check-branch-name.sh` and `scripts/package.sh` reject what they must and accept what they should | `sh scripts/test-checks.sh` |
| `just check-pins` | fails when `mise.toml` disagrees with the pins in the justfile, the workflows or `rust-toolchain.toml` (or lacks one of them, or lists a tool nothing else pins), or when a vendored manual asset no longer matches its recorded sha256. Reads the files as text: needs `uv` only, not mise, just's evaluator or the network | `uv run scripts/check_pins.py` |
| `just test-py` | `unittest` for `scripts/scrub_transcripts.py` and `scripts/make_goldens.py`, including one end-to-end run of the reference | `uv run python -m unittest discover -s scripts/tests` |
| `just check-plugin` | build the binary, then check the shipped plugin and marketplace: manifests parse, every hook command ends in `; exit 0`, every hook names a real subcommand of that binary. `claude plugin validate` also runs when the `claude` CLI is installed and is a printed `SKIP` otherwise (CI has none). Needs `cargo` and `uv` | `cargo build -p branch-tally` then `sh plugin/tests/check.sh target/debug/branch-tally` |
| `just lint-install` | ShellCheck on `install.sh` and its test, PSScriptAnalyzer on the `.ps1` files. A missing tool is a printed skip locally and an error when `CI` is set | `sh scripts/lint-install.sh` |
| `just test-install` | the installers' offline tests under `sh` and `pwsh`, with the same skip rule | `sh scripts/test-install.sh` |
| `just docs` | build the user manual with mdBook and mdbook-mermaid (pinned) into `docs/manual/book/`; a `SUMMARY.md` entry with no page fails it ([The user manual](#the-user-manual)) | `mdbook build docs/manual` |
| `just docs-serve` | serve the manual locally and rebuild on change, at `http://localhost:3000` | `mdbook serve docs/manual` |
| `just scrub --root <repo root> <src> <dest>` | scrub a real transcript tree into a committable fixture ([testing.md](testing.md#fixtures)) | `uv run scripts/scrub_transcripts.py …` |
| `just goldens [scenario] [--check]` | regenerate the fixture goldens from the reference; `--check` writes nothing and fails on a difference | `uv run scripts/make_goldens.py …` |
| `just next-version` | the version the next release would get, from the commits since the last tag (needs git-cliff at the pin) | `git-cliff --config cliff.toml --bumped-version` |
| `just release-notes [version]` | preview the notes for the unreleased commits; writes nothing | `git-cliff --config cliff.toml --unreleased --bump` |
| `just package <vX.Y.Z> <target> [binary]` | package a built binary as a release archive in `dist/` ([release.md](release.md#targets)) | `sh scripts/package.sh …` |
| `just token-record [args…]` | appends this branch's usage to `.tally/ledger.jsonl` with the installed release binary ([Token usage](#token-usage)) | `branch-tally record` |
| `just token-report [args…]` | this repo's own PR token report, from the installed release binary ([Token usage](#token-usage)) | `branch-tally report` |

Recipes run under PowerShell 7 on Windows and `sh` elsewhere, so a recipe line must be a plain tool
call that reads the same in both. Anything shell-specific goes in a private `_helper` recipe written
once under `[windows]` and once under `[unix]` (the `_require*` helpers that check for a tool are
the example). `test-scripts`, `lint-install` and `test-install` need `sh`, which Git for Windows
provides.

CI calls the same recipes, so a local `just ci` checks what CI checks.

**Pinned tools.** The `justfile` holds the versions of `rumdl`, `zizmor`, `gitleaks`, `git-cliff`
(read by `release.yml`, [release.md](release.md#versioning)), `mdbook` and `mdbook-mermaid`. CI
reads them back with `just --evaluate`, so local and CI runs use the same ones. Bump a version in
the justfile only. For gitleaks also bump the archive checksum in `ci.yml`, and for mdbook or
mdbook-mermaid the checksums in `docs.yml`; a mismatch fails the job. `just`, nextest and cargo-deny
are pinned in `ci.yml`.

**`mise.toml`.** `mise install` at the repository root sets up a dev machine with every one of
those pins (`rust`, `just`, `cargo-nextest`, `cargo-deny`, `gitleaks`, `git-cliff`, `rumdl`,
`zizmor`, `mdbook`, `mdbook-mermaid`). It is a **local convenience, not an authority**: the
justfile, the workflows and `rust-toolchain.toml` still decide what CI runs, and nothing in CI or
the hooks calls mise. `just check-pins` (part of `just ci`, and a CI step in the `scripts` job)
compares them as text and fails on any drift: a version that differs, a pinned tool missing from
`mise.toml`, or a tool in `mise.toml` that nothing else pins (so mise can't grow a pin of its own).
To bump a tool, change it where it is really pinned, then in `mise.toml` in the same commit.
Backends: `aqua:` (via the short registry name) for `just`, `cargo-deny`, `gitleaks`, `git-cliff`,
`rumdl`, `zizmor` and `mdbook`; `cargo:cargo-nextest` (not in the aqua registry; compiles from
source unless `cargo-binstall` is installed); `github:badboy/mdbook-mermaid` (no registry entry).
`uv`, `git`, `sh` and the installer linters are not pinned anywhere, so they are not in
`mise.toml`; install them yourself.

`mise.toml` also lists `rust`, even though `rustup` already reads `rust-toolchain.toml` and installs
that channel the first time `cargo` runs. The file stays the authority (it also carries the
components and profile, which `rustup` honours and a `mise.toml` entry would only duplicate), and CI
and the hooks use `rustup`. The `rust` line only lets `mise install` fetch the toolchain up front
with the other tools, and `check-pins` keeps its version equal to the channel in the toolchain file.

**Before the workspace exists (#3).** The Rust recipes (`test`, `lint`, `fmt`) find no
`Cargo.toml`, print "no Cargo.toml: Rust checks skipped" and exit 0. The CI Rust jobs do the same
with a `::notice::` annotation, so the required checks still report and start checking on their
own when the workspace lands. Markdown, workflow, secret and commit-message checks always run.

## Quality gates

Every gate runs twice: **locally, as a git hook**, so mistakes are caught before they leave the
machine, and **in CI**, so a skipped hook or a new contributor can't get around it. Both call the
same `just` recipes and `scripts/check-*.sh`.

| Gate | Local hook | CI (job name) |
| ---- | ---------- | ------------- |
| Secrets | `pre-commit`: `gitleaks git --pre-commit --staged --redact` | `secret-scan`: gitleaks over everything `HEAD` reaches (main plus the PR's commits, not other branches), pinned binary with a checked sha256 |
| Formatting | `pre-commit`: `just lint` (`cargo fmt --check`) | `lint`: `just lint` |
| Clippy | `pre-commit`: `just lint` (`cargo clippy -D warnings`) | `lint`: `just lint` |
| Markdown | `pre-commit`: `just lint-md` (`rumdl check`) | `lint-md`: `just lint-md` |
| Workflow security | `pre-commit`, when `.github/` is staged: `zizmor --offline` | `zizmor` (workflow `Workflow audit`): online audits when `.github/` or the justfile changes |
| Commit message | `commit-msg`: Conventional Commits | `commit-messages`: every commit in the PR; `pr-title`: the **PR title** |
| Branch name | `pre-push`: Conventional Branch, and refuses a push to `main` | `branch-name`: the PR's head branch |
| Tests | `pre-push`: `just test` | `test (ubuntu-latest)`, `test (macos-latest)`, `test (windows-latest)` and `msrv`: `just test` |
| Release build | — | `build` (workflow `Build`): on a PR that touches `build.yml`, `release.yml`, `scripts/package.sh`, `scripts/smoke-test.sh`, `cliff.toml`, `Cargo.lock` or `rust-toolchain.toml`, builds and smoke-tests all six release targets with a dummy version and publishes nothing ([release.md](release.md#proving-the-build-before-a-release)) |
| Pins | — | `scripts`: `just check-pins`, so `mise.toml` and the vendored manual assets can't drift from the pins and checksums they mirror (also in `just ci`) |
| Gate scripts | — | `scripts`: `just test-scripts` (so the commit and branch checks, the packaging script and the installer gates, can't silently stop rejecting) and `just test-py` (the scrubber and goldens generator) |
| Plugin | — | `plugin`: `just check-plugin`, so a hook that could block a Claude Code session, or name a subcommand that does not exist, fails the PR. Manifest validation by `claude plugin validate` is skipped there (no `claude` CLI on the runner) and says so |
| Installers | — | `lint-install` and `test-install (<os>)` (workflow `Installers`), only when an installer, its tests or the workflow change ([release.md](release.md#proving-an-install-works)) |
| User manual | — | `docs` (workflow `Docs`): `just docs`, only when `docs/manual/`, the justfile or the workflow change. Not a required check, for the same reason as `zizmor` ([The user manual](#the-user-manual)) |
| Rust path filter | — | `changes` (workflow `CI`): `dorny/paths-filter` decides whether a PR touches Rust-relevant files and the Rust jobs skip their steps when it doesn't ([Path filtering](#path-filtering)) |
| Dependencies | — | `deny`: `cargo deny check` (licences, advisories, duplicates), once `deny.toml` exists |
| New vulnerable dependencies | — | `dependency-review`: fails a PR that adds a dependency with a known vulnerability of moderate severity or worse |
| Edit-time lint | Claude Code `PostToolUse` hook: `rustfmt --check` on a `.rs` file, `rumdl check` on a `.md` file | — |

**Dependabot** keeps the dependencies current. `.github/dependabot.yml` groups updates (one weekly
PR per ecosystem), waits a week after a release before proposing it, and prefixes the commit and PR
title with `ci(deps):` for workflow actions and `build(deps):` for crates, so the PR-title check
passes. Its `dependabot/...` branches are exempt from the Conventional Branch check, since the bot
names them. Dependabot alerts and security updates are repository settings (see the table above).

While there is no `Cargo.toml`, the Rust jobs (`lint`'s Rust half, the `test` matrix, `msrv` and
`deny`) pass with a "Rust checks skipped" notice rather than not running, so a required check is
never stuck waiting. Once `Cargo.toml` exists, `msrv` fails unless it declares a `rust-version`.

Tests run on `pre-push` rather than `pre-commit` so that a red commit is possible. Test-first
produces one on purpose, and the commit history should show the red test landing before the code.

Hooks are enabled per clone:

```bash
git config core.hooksPath .githooks
```

The path is relative, so each worktree runs **its own branch's** hooks. `worktree-bootstrap`
checks this.

Don't pass `--no-verify`. If a hook fails, fix what it found. A tool missing from `PATH` makes the
hook fail with an install hint. It never silently skips the check.

### Path filtering

A PR that changes only docs, Markdown, `.claude/` or another workflow doesn't need the Rust suite.
The `changes` job in `ci.yml` runs [`dorny/paths-filter`](https://github.com/dorny/paths-filter)
(pinned, `contents: read` and `pull-requests: read`, no checkout) and publishes `code`, true when
the PR touches any of:

- `crates/**` (which holds the Rust sources, the tests under `crates/*/tests/` and the bundled
  data), `tests/**`, `Cargo.toml` (root or any crate), `Cargo.lock`, `rust-toolchain.toml`;
- `rustfmt.toml`, `clippy.toml`, `.config/nextest.toml`, `deny.toml`, `.cargo/**`;
- `plugin/**` and `.claude-plugin/**`, which the `plugin` job checks against the built binary;
- `justfile`, `scripts/ci-detect.sh`, `scripts/ci-install-rust.sh` and `ci.yml` itself, because a
  change to the gate must exercise the gate.

Anything else (`docs/**`, `*.md`, `.claude/**`, the other workflows, the other `scripts/**`) is not
code. The filter list is in `ci.yml`; change it there.

**The jobs still run, and skip their steps.** `lint`, `test (<os>)`, `msrv` and `deny` are
required checks. A matrix job skipped by a job-level `if:` reports one check under its unexpanded
name, so `test (ubuntu-latest)` would never report and the PR would wait forever. So these jobs
always start (`if: ${{ !cancelled() }}`), and `scripts/ci-detect.sh` turns `rust=false` when
`CODE_CHANGED=false`, with a `::notice::` ("Rust checks skipped"), the same path as before the
workspace existed (#3). Each Rust step is gated on that output. `scripts/test-checks.sh` proves the
script skips only on the exact string `false`.

**What always runs:** `lint-md`, `scripts`, `secret-scan`, the commit, branch and PR-title checks,
`dependency-review`, `zizmor` (when `.github/` changes) and `docs` (when the manual changes). On a
push to `main` the filter step is skipped, `code` is empty, and everything runs. If `changes` itself
fails, `code` is empty too and everything runs: only a definite `false` skips, so a broken filter
fails safe, never green.

### Writing workflows that pass zizmor

[zizmor](https://docs.zizmor.sh/) audits `.github/` for GitHub Actions security problems. Fix its
findings; don't silence them. Every workflow is held to these rules:

- **Pin every action to a full commit SHA**, with its version as a trailing comment.
- **Start from `permissions: {}`** at the top, and give each job only what it uses.
- **`persist-credentials: false` on every checkout** unless a job must push.
- **No `${{ }}` inside `run:`.** Pass values through `env:` and use `"$NAME"`.
- **Name the secrets a reusable workflow gets** instead of `secrets: inherit`.

### The user manual

The user manual is an [mdBook](https://rust-lang.github.io/mdBook/) in `docs/manual/`
([0003-user-manual.md](../decisions/0003-user-manual.md)). `book.toml` is at its root, the pages
and `SUMMARY.md` are in `src/`, and the build goes to `docs/manual/book/` (gitignored, and excluded
from `rumdl`). Diagrams are Mermaid fenced blocks, rendered by the `mdbook-mermaid` preprocessor.
Its two assets, `mermaid.min.js` and `mermaid-init.js`, are committed beside `book.toml`, outside
`src/`.

- **URL.** The manual is served at the apex, `https://branch-tally.io/` (DNS is done; the custom
  domain is set by hand in the repository's Settings, Pages). So `site-url` in `book.toml` is `"/"`.
  No `CNAME` file is shipped: with Pages deploying from Actions (`build_type: workflow`) GitHub
  ignores a `CNAME` in the artifact and takes the domain from the setting, so the file would only be
  a second copy to drift. Links in `README.md` and `docs/manual/src/` use the apex.
- **Zoomable diagrams.** Every Mermaid diagram can be zoomed and panned, in every mdBook theme.
  `svg-pan-zoom.min.js` is vendored beside `book.toml` (svg-pan-zoom 3.6.2, BSD 2-Clause: licence in
  `svg-pan-zoom.LICENSE`, source URL and checksums in the `book.toml` header,
  `svg-pan-zoom.min.js.sha256` for `just check-pins` to verify). `svg-pan-zoom-init.js` and
  `svg-pan-zoom.css` are ours, loaded through `additional-js` and `additional-css`; the generated
  `mermaid-init.js` is untouched, so refreshing it with `mdbook-mermaid install` loses nothing. The
  diagram stays its normal size and the page scrolls normally until the reader uses the corner
  controls or Ctrl/Cmd + wheel; once zoomed in, dragging pans and the frame grows to 70vh. The init
  script waits for Mermaid's asynchronous render to settle before attaching (attaching to a
  half-rendered `<svg>` makes svg-pan-zoom throw). To update the library: replace the file from the
  npm tarball, write the new sha256 to `svg-pan-zoom.min.js.sha256`, and update the `book.toml`
  header and `svg-pan-zoom.LICENSE`.
- **Versions.** `mdbook_version` and `mdbook_mermaid_version` in the justfile. mdBook 0.5 changed
  the preprocessor API, so `mdbook-mermaid` must be a release built against it (0.17 and later).
  mdBook prints a "built against version 0.5.0 of mdbook" warning when the patch versions differ;
  that is harmless within 0.5. To move the pin, bump both versions and both checksums in `docs.yml`,
  then run `mdbook-mermaid install docs/manual` with the new binary and commit the two refreshed
  assets.
- **`create-missing = false`.** A `SUMMARY.md` entry that points at a missing page fails the build
  instead of quietly creating an empty page.
- **CI.** The `Docs` workflow (`docs.yml`) builds the book on a PR that touches `docs/manual/**`,
  the justfile or the workflow, installing the prebuilt release binaries against a checked sha256
  like gitleaks. On a push to `main` that touches `docs/manual/**`, and on `workflow_dispatch`, it
  also publishes the build to GitHub Pages (`actions/upload-pages-artifact` then
  `actions/deploy-pages`). Only the `deploy` job holds `pages: write` and `id-token: write`. Pages
  is set to `build_type: workflow` and the site is `https://branch-tally.io/`.
  The `docs` job is path-filtered, so it is not a required check; requiring it would leave every PR
  that doesn't touch these paths waiting on a check that never reports.
- **`just ci` includes `docs`**, because `just ci` is every gate CI runs. It needs `mdbook` and
  `mdbook-mermaid` on `PATH` and fails with an install hint if either is missing.
- **Docs upkeep.** A PR that changes a CLI flag, a config key or an install method updates
  `docs/manual/` in the same PR ([Pull requests](#pull-requests)).

### When gitleaks stops a commit

- **A real secret:** unstage it and move it somewhere outside the repository. If it was ever pushed
  or pasted anywhere, treat it as leaked and rotate it.
- **A false positive** (a fixture's fake key, a UUID that looks like a token): add a
  `gitleaks:allow` comment on that line, or add the fingerprint to `.gitleaksignore`. Either way,
  say why in the commit message.

Fixtures deserve extra care here. They are derived from real transcripts, and a secret that
survived scrubbing is a real leak, not a false positive ([testing.md](testing.md#fixtures)).

## Pull requests

Every PR:

- has a Conventional Commit title;
- links its issue with `Fixes #N`, or `Refs #N` when it only partly resolves it;
- carries the label its branch type implies (`enhancement`, `bug`, `documentation`);
- records its own token usage (dogfooding, see below).
- **updates the manual** when it changes a CLI flag, a config key or an install method: edit the
  affected page under `docs/manual/src/` in the same PR. The manual documents what ships on
  `main`, so a feature still in flight is documented in the PR that lands it. A reviewer treats a
  flag, key or install-method change with no `docs/manual/` change as incomplete. The
  `docs_drift` test ([testing.md](testing.md#docs-drift)) enforces the CLI and config half: a flag,
  subcommand or config key missing from `cli.md` or `config.md` fails `just test`. Install methods
  stay a review responsibility.

`/create-gh-pr` handles the first four. Tick test-plan boxes honestly. An unticked box tells the
reviewer something. A falsely ticked one is a lie they can't see.

### Token usage

This project measures its own PRs with branch-tally itself (#41):

```bash
just token-record                     # branch-tally record: append to .tally/ledger.jsonl
just token-report --pr --from-ledger  # branch-tally report: fill the PR table from the ledger
```

`/create-gh-pr` runs `record`, commits `.tally/ledger.jsonl` (and `.tally/captures.jsonl` if
present) to the PR branch, pushes, then fills the table from the ledger. Both ledger files merge
with `merge=union` (`.gitattributes`). `just token-report --pr` still works and reads the live
transcripts directly. The [ledger chapter](../manual/src/ledger.md) describes the files.

This repository enables the published `branch-tally-capture` and `branch-tally-record` plugins in
`.claude/settings.json`, so sessions here capture attribution and record on SessionEnd. The
SessionEnd hook gets only 1.5 seconds by default; raise `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS`
as [plugin/README.md](../../plugin/README.md#sessionend-time-limit) describes.

It runs the **installed release binary**, not `cargo run`, so the report comes from a published
build rather than the branch under review. Install the latest
[release](https://github.com/IAmSkweetis/branch-tally/releases/latest) archive for your platform
and put `branch-tally` on `PATH`. The recipe stops with that hint if it is missing.

`report` attributes by the session's claimed branch by default. `--strict` is the naive
`gitBranch` filter. `--timeline` attributes each call to the branch the session was on at that
moment (a third mode, exclusive with `--strict`); `--work-branch <REGEX>` (timeline only) names
which branches count as work branches, matched unanchored as written, so `^(feat|fix)/` works.
The default is every branch except the repository's default branch and `worktree-agent-*`.

The same choices are `--mode claim|strict|timeline` and `--format md|json`; `--strict`,
`--timeline`, `--md` and `--json` stay as shorthands, and a shorthand cannot be combined with its
long form, even when they agree (exit 2). `--cost` pairs with `--no-cost`, `--redact` with
`--no-redact` and `--keep-names` with `--redact-names` (#9; the manual's
[CLI reference](../manual/src/cli.md#redaction) describes them). `--work-branch` is a usage error
unless the *effective* mode is timeline, whether that came from `--mode timeline`, `--timeline` or a
config `mode = "timeline"`; a work-branch set only in the config file is ignored outside timeline
mode. Defaults live in a user config file (flag > file > default); the manual's
[Config reference](../manual/src/config.md) has its location and keys, `branch-tally config init`
writes a starter file, and `branch-tally config show` prints what applies.

It must run on the machine that did the work, and **while every agent worktree still exists**
until issue #6 removes that dependency. Report the number as a floor ("at least $X").
`reference/token_report.py` is no longer run for this. Only the golden generator (`just goldens`)
still runs it, as the parity oracle.

## Related pages

- [agent_workflow.md](agent_workflow.md): how agent-run work fits this loop
- [testing.md](testing.md): test-first in practice, fixtures, the false-green traps
- [release.md](release.md): how a release is cut from `main`
- [project_layout.md](project_layout.md): where a new file belongs
