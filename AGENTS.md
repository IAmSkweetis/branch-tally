# branch-tally agent instructions

branch-tally reports what a branch or pull request cost in AI-agent tokens. It reads the agent
harness's local transcripts, assigns each recorded API call to the git branch the work was for, and
publishes a cost table on the PR. That includes work done by subagents in git worktrees. Claude Code
is the first harness supported; others follow. It is written in Rust and ships as a CLI, a Claude
Code plugin/skill and a GitHub Action.

The roadmap is GitHub issue #16. Each item is a sub-issue labelled `roadmap`.

## Governance

### I. Test-first, red to green

We develop test-first. Write the test before the code, run it, and **watch it fail for the reason
you intended**. Then write the smallest code that makes it pass, and watch it pass. A test never
seen red has not been shown to test anything. A compile error is not red: stub the signature
(returning a wrong value or `todo!()`) so the test compiles and fails at runtime. Never weaken an
assertion, widen a tolerance, accept a snapshot unread, or `#[ignore]` a test to reach green.

### II. Honest numbers

The product is a number people will trust, so it must say what it could not see. A record that
can't be attributed with certainty is **counted and reported** as uncertain. It is never dropped
silently, and never quietly assigned to a branch on a guess. A partial total is labelled as a floor
("at least $X"). A heuristic that widens what the tool claims needs a test proving the claim and a
line in the report saying the heuristic was used.

### III. Transcript privacy

Transcripts contain prompts, code and secrets. branch-tally reads only the fields it needs: usage,
model, ids, timestamps, branch, cwd and attribution. **Message content never leaves the machine.**
It must not appear in fixtures, the ledger, PR output, logs or error messages. Fixtures are built
from scrubbed transcripts only (see [testing.md](docs/references/testing.md#fixtures)). Redaction
(#9) hides identifiers on request. Content is never emitted in the first place.

## Tech stack and constraints

The official Rust documentation is the default authority: [The Rust Book][book], the
[API Guidelines][api], the [Cargo Book][cargo] and the [Clippy lint list][clippy]. Follow them
wherever this page is silent.

- **Language**: Rust, stable channel, edition 2024. The toolchain is pinned in
  `rust-toolchain.toml` once the workspace exists (#3). The language decision is #1.
- **Workspace**: a library crate, `branch-tally-core`, holds all logic: sources, attribution,
  pricing, ledger and rendering. A thin binary crate, `branch-tally`, holds the CLI. The full layout
  and the rules for where a file belongs are in
  [project_layout.md](docs/references/project_layout.md).
- **Pipeline direction**: each stage only consumes the one before it: source → attribution → ledger
  → pricing → render. A source never decides a branch, attribution never formats output, and a
  renderer never re-derives totals. Anything that touches git, `gh`, the network or the clock sits
  behind a trait in core, so tests inject a fake.
- **Errors**: library code returns typed errors. Bad *input* (a malformed JSONL line, an unknown
  model, a missing field) is counted and reported, never a panic. No `unwrap()`/`expect()` on
  transcript data outside tests.
- **Warnings**: `rustc` and Clippy warnings are fixed, not silenced. CI runs Clippy with
  `-D warnings`. When the flagged code is deliberate, suppress at the narrowest scope with
  `#[expect(lint_name, reason = "...")]`, not `#[allow]`. `expect` fails if the lint stops firing,
  so the suppression can't outlive its reason. Never disable a lint workspace-wide to quiet one
  case.
- **Formatting**: `rustfmt` with the repository's `rustfmt.toml`. Line endings are LF everywhere,
  enforced by `.gitattributes`.
- **Dependencies**: every new crate, feature flag or version bump goes through the Workspace Config
  Steward (see [agents.md](docs/references/agents.md)). We ship a small static binary and read
  private data, so a dependency is a supply-chain and size decision, not a convenience.
- **Paths**: Windows is a first-class platform. Never build a path by string concatenation, and
  compare transcript slugs case-insensitively. Transcripts contain both `D:\…` and `/d/…` forms of
  the same cwd ([transcript_format.md](docs/references/transcript_format.md)).
- **Markdown**: linted by `rumdl` against `.rumdl.toml`. Wrap prose at 100 columns. Fix a finding;
  don't silence it.

[book]: https://doc.rust-lang.org/book/
[api]: https://rust-lang.github.io/api-guidelines/
[cargo]: https://doc.rust-lang.org/cargo/
[clippy]: https://rust-lang.github.io/rust-clippy/stable/

## Running the tests

```bash
just test          # cargo nextest run --workspace, plus doctests
just ci            # fmt check, clippy, tests, doctests, rumdl, gitleaks: every gate CI runs
```

Until the Cargo workspace exists (#3), the Rust recipes print "no Cargo.toml: Rust checks skipped"
and exit 0. That is a skip, not a green run.

**Always report the test count alongside the exit code.** Exit 0 with zero tests run is a false
green. A filter that matches nothing, a test file outside a crate's `tests/` directory, or a test
behind a disabled feature all produce one. `cargo nextest` fails a run that matches no tests, but
it **does not run doctests**, which is why `just test` runs them separately. The `rust-tests` skill
covers the traps and the snapshot rules.

`just` is the entrypoint for every script, gate and build. `just` on its own lists the recipes, and
[dev_workflow.md](docs/references/dev_workflow.md#commands) describes them.

## Development workflow and quality gates

We use **trunk-based development**. `main` is the trunk: always green and always releasable.
Branches are short-lived, cut from `main`, and land back on it through a pull request that CI
gates. The only allowed merge method is a **squash merge**, so the PR title becomes the commit on
`main` and must itself be a valid Conventional Commit. Never commit directly to `main`, never
force-push, and never merge work without the PR. Releases are tags cut from `main`, never from a
long-lived branch ([release.md](docs/references/release.md)).

Branch names follow [Conventional Branch](https://conventionalbranch.org/) and commit messages
follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/). Both are mandatory
on every branch and every commit. **Read [dev_workflow.md](docs/references/dev_workflow.md) before
your first branch or commit of a session.** It has the allowed types, how they map onto each other,
and what has to be green before anything lands.

**Gates are enforced twice**: by git hooks locally (`git config core.hooksPath .githooks`) and by
CI on every PR. Don't pass `--no-verify`. If a hook fails, fix what it found.

**Complex work uses subagents in worktrees.** `/feature` interviews you, cuts the feature branch,
dispatches agents into worktrees and merges their work back. `/create-gh-pr` opens the PR and
records its token usage. `/worktree-cleanup` removes the worktrees **after** the PR merges. An
agent commits in its worktree and never pushes. Small changes don't need the machinery. See
[agent_workflow.md](docs/references/agent_workflow.md#when-an-agent-is-the-wrong-tool).

## References

Reference pages live in `docs/references/`, indexed in
[docs/references/README.md](docs/references/README.md). They are **not** loaded into context
automatically. When a task touches a page's area, read the page before changing anything. Each one
records decisions and gotchas you can't recover from the code. When you change what a page
describes, update the page. When you write a new page, add a row here and in the index.

| Page | Read it when |
| ---- | ------------ |
| [project_layout.md](docs/references/project_layout.md) | Adding a file and deciding where it goes: which crate, which module, where its tests and fixtures live. |
| [dev_workflow.md](docs/references/dev_workflow.md) | Naming a branch, writing a commit message, running or adding a `just` recipe, opening a PR, or asking what has to be green before something lands. |
| [testing.md](docs/references/testing.md) | Writing or running a test, adding a fixture, touching a snapshot, or checking parity against the Python reference. |
| [agent_workflow.md](docs/references/agent_workflow.md) | Dispatching a subagent, running work in parallel worktrees, reading an agent report, or routing an escalation. |
| [agents.md](docs/references/agents.md) | Deciding which agent owns a file, or what a given agent may write. |
| [release.md](docs/references/release.md) | Cutting a release, how versions are chosen, or how artifacts are built and published. |
| [transcript_format.md](docs/references/transcript_format.md) | Parsing Claude Code transcripts, or attributing records to branches, worktrees or subagents. |
