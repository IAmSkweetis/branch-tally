# Testing

How tests are written and run here, and what makes a passing run trustworthy. Governance I in
`AGENTS.md` is the rule. This page covers how it works in Rust. The `rust-tests` skill is the
command-level checklist.

## Red to green, in Rust

1. **Write the test** where it belongs ([project_layout.md](project_layout.md#tests)).
2. **Make it compile, and only that.** Add the type or function signature with a body that returns
   a wrong value or calls `todo!()`. A compile error is not red: it shows the code doesn't exist,
   not that the test can detect it being wrong.
3. **Run it and watch it fail for the reason you intended.** Read the failure message. A test that
   fails for a different reason (a panic in setup, a missing fixture) hasn't been shown to test
   anything yet.
4. **Write the smallest code that makes it pass.** Run it and watch it pass.
5. **Refactor** with the test green. Run the whole suite, not just the one test.

Commit the red test on its own when that helps, for example when someone else will implement it.
Tests run on `pre-push`, not `pre-commit`, for exactly that reason.

Never reach green by weakening an assertion, widening a tolerance, adding `#[ignore]`, or accepting
a snapshot you didn't read. If the test is wrong, fix it deliberately and say so in the commit.

### Bugs start with a failing test too

A `fix:` PR begins with a test that reproduces the bug and fails on `main`. For an attribution bug,
that is nearly always a new fixture scenario. #6 is the model: its fixture is "an orchestrator on
`feat/x` dispatches three worktree subagents, then the worktrees are removed".

## Running

```bash
just test                 # nextest over the workspace, then doctests
just test attribution     # nextest filter, for a fast inner loop
just ci                   # every gate CI runs
```

[cargo-nextest](https://nexte.st/) is the runner. It needs a one-time install
(`cargo install cargo-nextest --locked`, or `cargo binstall cargo-nextest`).

### False greens

**Always report the test count with the exit code.** Each of these exits 0 while testing less than
you think:

| Trap | Guard |
| ---- | ----- |
| A filter that matches no tests | nextest fails a run with no tests. Plain `cargo test <filter>` does **not**, so don't use it to prove green. |
| Doctests | nextest doesn't run them. `just test` runs `cargo test --doc` separately. |
| A test file that is never compiled (outside `tests/`, or a module never declared with `mod`) | Compare the count with `main`'s. A new test file should raise it. |
| A test behind a cargo feature that isn't enabled | CI runs `--all-features` once. Locally, name the feature. |
| `#[ignore]` | Not allowed on `main` without a linked issue in the attribute: `#[ignore = "#NN: reason"]`. |
| An auto-accepted snapshot | See [Snapshots](#snapshots). |
| A fixture that no longer exercises the path it was written for | Each fixture's `README.md` states the behaviour it covers. The test asserts a count that proves that path ran. |

## Fixtures

Fixtures are transcript trees laid out exactly as a harness writes them, under
`tests/fixtures/<harness>/<scenario>/`. Each scenario directory has a short `README.md`: what
situation it reproduces, which issue it came from, and what a correct report says about it.

**Fixtures are always scrubbed.** Governance III applies with extra force here, because a fixture
is committed and published.

- Derive fixtures from real transcripts only through the scrubbing tool (#2). It keeps only the
  fields branch-tally reads and replaces every content field with a placeholder.
- **Never hand-copy lines** from `~/.claude/projects/` into a fixture.
- Rewrite user-specific paths to a neutral root (`/repo`, `C:\repo`). Keep both separators where a
  scenario depends on them.
- Hand-written synthetic fixtures are fine, and often better: smaller and more targeted. They still
  get a `README.md`.

Before committing a fixture, grep it for anything that looks like prose, a home directory or a
token. `gitleaks` runs on commit, but it catches secrets, not private text.

## Parity with the Python reference

`reference/token_report.py` is the oracle for the first Rust port (#2, #4). Its `--json` output on
each fixture is stored as a golden file. Rust output must match it, **except** where an issue
deliberately changes behaviour (#6, #7). Each deliberate divergence is listed in the golden test,
next to the issue number that justifies it. Once Rust is the source of truth, the goldens become
ordinary snapshots and the reference script is retired.

## Snapshots

Rendered output (Markdown tables, JSON) is checked with snapshot tests. The proposed crate is
`insta`, confirmed in #2. Snapshots are the easiest place to fake a green:

- CI runs with snapshot updates disabled, so a changed snapshot fails.
- A new or changed snapshot is **reviewed line by line** before it is accepted. Accepting
  everything wholesale (`cargo insta accept`) turns red into green without anyone reading it.
- Snapshot diffs belong in the PR. A reviewer should see the output change.

## Related pages

- [project_layout.md](project_layout.md#tests): where each kind of test lives
- [transcript_format.md](transcript_format.md): the input the fixtures reproduce
- [dev_workflow.md](dev_workflow.md#quality-gates): where tests sit among the gates
