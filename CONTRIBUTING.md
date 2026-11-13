# Contributing to branch-tally

Thanks for helping. This page gets a working checkout set up and explains what has to be green
before a change lands. The rules behind it live in [AGENTS.md](AGENTS.md) and
[docs/references/](docs/references/README.md).

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Found a
vulnerability? Don't open an issue: report it privately as described in [SECURITY.md](SECURITY.md).

## Prerequisites

| Tool | Why | Install |
| ---- | --- | ------- |
| [Rust](https://rustup.rs) (stable, via rustup) | Builds and tests the code. `rust-toolchain.toml` pins the version (1.99.1). | `rustup` picks up the pin on its own |
| Visual Studio Build Tools, "Desktop development with C++" (Windows only) | The `x86_64-pc-windows-msvc` toolchain links with MSVC's `link.exe` and the Windows SDK. Without them every build that links fails, and Git Bash's unrelated `/usr/bin/link` makes the error misleading. Install them before `cargo install` anything. | `winget install Microsoft.VisualStudio.BuildTools --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| [just](https://just.systems) 1.56 or newer | Runs every recipe, hook and CI gate | `cargo install just`, `scoop install just`, `brew install just` |
| [cargo-nextest](https://nexte.st) | The test runner | `cargo install cargo-nextest --locked` |
| [uv](https://docs.astral.sh/uv/) | Runs the pinned `rumdl` and `zizmor` through `uvx`, and the Python reference script | see the uv install guide |
| [gitleaks](https://github.com/gitleaks/gitleaks) | Secret scanning in `pre-commit` and CI | `scoop install gitleaks`, `brew install gitleaks` |
| PowerShell 7 (Windows only) | The `justfile` runs recipes under `pwsh` on Windows | `winget install Microsoft.PowerShell` |
| Git for Windows (Windows only) | Provides the `sh` that git hooks and `scripts/` run under | <https://gitforwindows.org> |

You don't install `rumdl` or `zizmor` yourself. The `justfile` pins their versions and runs them
with `uvx`, so your run and CI's run agree.

## Set up a checkout

```bash
git clone https://github.com/IAmSkweetis/branch-tally.git
cd branch-tally
git config core.hooksPath .githooks   # enable the git hooks; once per clone
just                                  # list the recipes
just ci                               # run every gate CI runs
```

`just ci` should exit 0. Until the Cargo workspace exists (#3), the Rust checks print
`no Cargo.toml: Rust checks skipped` instead of running. That is a skip, not a pass.

If a hook or recipe says a tool is missing, it prints an install hint. Install the tool. Don't
bypass the hook with `--no-verify`.

## Everyday commands

| Command | Does |
| ------- | ---- |
| `just test [filter]` | nextest over the workspace, then doctests |
| `just lint` | `cargo fmt --check` and Clippy with warnings denied |
| `just fmt` | format the Rust code |
| `just lint-md` / `just fmt-md` | check or rewrap Markdown (100 columns) |
| `just secret-scan` | gitleaks over the full history of every local branch |
| `just audit-workflows` | zizmor over `.github/` |

The full list, and what CI runs on top, is in
[dev_workflow.md](docs/references/dev_workflow.md#commands).

## Making a change

1. **Find or open an issue.** Planned work is tracked in the
   [`v1` milestone](https://github.com/IAmSkweetis/branch-tally/milestone/1). A small change can
   skip this and say why in the PR.
2. **Branch from an up-to-date `main`**, named per
   [Conventional Branch](https://conventionalbranch.org/): `feat/issue-4-report-command`,
   `fix/…`, `chore/…`. The `pre-push` hook rejects anything else, and any push to `main`.
3. **Write the test first** and watch it fail for the reason you intended, then make it pass. See
   [testing.md](docs/references/testing.md).
4. **Commit** with [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) messages.
   The `commit-msg` hook checks them.
5. **Open a pull request** whose title is also a Conventional Commit. PRs are squash-merged, so
   the title becomes the commit on `main`. Link the issue with `Fixes #N`, or `Refs #N` for partial
   work. A small change with no issue says `No issue: <reason>`.
6. **CI must be green.** It runs the same gates as the hooks, plus tests on Linux, macOS and
   Windows, the MSRV, dependency review and `cargo deny`.
7. **Get one approval.** [CodeRabbit](https://docs.coderabbit.ai/) reviews every non-draft PR.
   Reply to or resolve each of its threads. It approves once they are resolved and the latest
   commit is reviewed, and that approval, or a maintainer's, is what `main` requires.

[dev_workflow.md](docs/references/dev_workflow.md) covers the allowed types, the branch-to-commit
mapping and every quality gate in detail.

## Transcripts and privacy

branch-tally reads Claude Code transcripts, which contain prompts, code and secrets. **Never commit
a real transcript or any part of one**: not in a fixture, a test, an issue or a PR. Fixtures are
built from scrubbed transcripts only; see
[testing.md](docs/references/testing.md#fixtures). If gitleaks flags something in a fixture, treat
it as a real leak until proven otherwise.

## Licence

By contributing you agree that your contribution is licensed under the [MIT License](LICENSE).
