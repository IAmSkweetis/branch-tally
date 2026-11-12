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

- Derive fixtures from real transcripts only through the scrubbing tool (#2), which lives in
  `scripts/` like every Python script here. It keeps only the fields branch-tally reads and
  replaces every content field with a placeholder.

  ```bash
  just scrub --root <your repo root, as recorded> <slug dir or projects root> <dest>
  ```

  `--root` is the one thing it can't guess: every `cwd` under it becomes `C:\repo` or `/repo`
  (keeping the `D:\…` versus `/d/…` form and the worktree subpath), and anything else becomes
  `C:\other` or `/other`. Kept: `type`, `requestId`, `uuid`, `sessionId`, `agentId`,
  `isSidechain`, `timestamp`, `cwd`, `gitBranch`, `attributionSkill`, `attributionAgent`,
  `message.model` and the numeric `message.usage` fields (the set `sources/claude_code/parse.rs`
  reads). `message.content` becomes `"PLACEHOLDER"`; every other key is dropped, not copied.
  Ids become `s1`, `u1`, `req_1`, `a1`, equal in giving equal out (dedupe depends on it), and the
  files and directories are renamed to match. A malformed line, or one whose kept field has the
  wrong type, is dropped and only counted on stderr. The run ends by re-reading the output against
  an allowlist and deleting it if anything else survives. Review the output anyway: a branch name
  or a skill name is kept because it is read, and it can still be private.
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

**Generating the goldens.** `just goldens [scenario]` (`scripts/make_goldens.py`). Each scenario
directory under `tests/fixtures/claude-code/` is itself a projects root, with a
`goldens/scenario.json` manifest: `repo_slug` (the main checkout's slug), `worktrees`
(`{name, branch}` pairs, created as real git worktrees so the script's `git worktree list`
recovery sees them; leave one out to model a removed worktree) and `claim_branches`. The script
can't be pointed at a fixture, so the generator builds a throwaway home (`HOME` and `USERPROFILE`)
with a throwaway git repo, copies the scenario's directories in under that repo's real slug
(`repo_slug` prefix replaced, so sibling and worktree relationships survive), and runs the
reference once per pair. It writes, next to the manifest:

- `goldens/strict.json`: `{"<raw gitBranch>": <--strict --json payload>}`, one key per distinct
  raw `gitBranch` in the usage-bearing records the reference reads (`"(none)"` for records with
  none). These feed the Phase 0 parity test: calls kept and token totals, per session and model.
- `goldens/claim.json`: the same shape for the manifest's `claim_branches`, run without `--strict`.
  They feed `rust_report_matches_the_reference_claim_goldens` (#4), which compares the full
  `render::json` report against every `claim.json` key (claim mode) and every `strict.json` key
  (strict mode) as parsed JSON, through `report::build`. Worktrees are built as git lists them (main
  checkout on `main`, then `.claude/worktrees/<name>`). The reference recovers unstamped
  main-checkout records onto that `main`; Rust doesn't (#37, listed in `CLAIM_DIVERGENCES`).
  Origins are compared lowercased: the reference lowercases them and the generator maps them back to
  fixture spelling, which is not behaviour. One output divergence is listed (#5, below).

A pair the reference answers with "No transcript records found" is recorded as
`{"error": "no_records"}`, never dropped. Any other failure aborts the run. Session `origin` is
mapped back to the fixture's own directory name (or `"main"`), and the run fails if a temp path,
slug or user name reaches a golden. Output is sorted, LF and deterministic, so regenerating is a
no-op diff; `just goldens --check` fails on any difference (and on a scenario without a manifest
being asked for by name). Scenarios without a manifest are reported on stderr, not skipped
silently, and a run that finds no manifest at all fails.

The Python tooling's own tests are `just test-py` (stdlib `unittest`, part of `just ci`).

The script is **immutable**. It is run, never edited, not even to fix a bug: a bug in it is a
deliberate divergence, listed like any other. When the reference crashes or is plain wrong on a
whole scenario, list that in the manifest instead of working around it:

```json
"reference_divergence": {"issue": "#3", "reason": "reference crashes on a bare 42"}
```

The generator then does not run the reference for that scenario and writes no goldens. It prints a
stderr line naming the scenario, issue and reason, and `just goldens --check` fails if goldens
exist for it. Both keys are required, non-empty strings. The scenario's Rust test asserts the
expected behaviour directly, next to the same issue number. `malformed-and-keyless` is the first
(a bare `42` line makes the reference raise `AttributeError`; Rust counts it as malformed).

**The parity tests** are `crates/branch-tally-core/tests/parity_claude_code.rs`:

- `rust_matches_the_reference_on_calls_and_tokens` (Phase 0) walks every scenario with a manifest,
  reads it with the Claude Code source (the scenario directory is the projects root, repo
  `C:\repo`), groups the Records by raw `gitBranch` (absent: `"(none)"`) and compares each key
  with `strict.json`: `totalCalls`; per session id, calls and tokens; per model, calls, input,
  both cache writes, cache read, output and thinking. Tokens follow the reference's
  `Record.tokens` (input + 5m + 1h + read + output; thinking is inside output). A `no_records`
  key must match zero Rust records. Cost, origin, first/last and agents are not compared. The
  test pins the number of scenarios (20) and branch keys (50) compared, so it cannot go green
  on nothing, and fails on a scenario with a manifest but no goldens. Adding a scenario means
  bumping `EXPECTED_SCENARIOS`, `EXPECTED_BRANCH_KEYS` and `EXPECTED_CLAIM_PAIRS`.
- Divergences are two lists in the test: `REFERENCE_DIVERGENT_SCENARIOS` (the manifest's
  `reference_divergence`; currently `malformed-and-keyless`, #3) and `DIVERGENCES` (a scenario
  and branch key where Rust deliberately differs, with its issue). A listed key must still
  differ, so an entry cannot outlive its reason. `DIVERGENCES` is empty today.
- Output divergence #5 (price table and floor): the JSON report has three top-level keys the
  reference lacks, `totalCostIsFloor`, `unpricedModels` and `priceTable` (`asOf`, `source`), and
  the Markdown report labels a partial total "at least $X" and states the price table date in the
  footnote. The three keys are listed in `PRICE_DIVERGENT_KEYS` in `parity_claude_code.rs`,
  and `branch-tally/tests/report.rs` (inline in `tests/render.rs`). Each test asserts the
  keys are present and consistent with the per-model costs, removes only those, then compares
  everything else to the goldens exactly. Markdown is not compared with the reference.
- Output divergence #6 (subagent rollup): when `Summary.subagents` is not empty, the JSON report has
  a top-level `subagents` object (`calls`, `tokens`, `dispatched`, `workspaces`, `worktrees`,
  `unattributedCalls`, `unattributedTokens`, `unlinkedInTotalCalls`, `unlinkedInTotalTokens`,
  `rows[]` of `workspace`, `workspaceKind` (`main`, `worktree` or `unknown`, the last for a missing
  or outside-repo cwd), `isWorktree`, `subagent`, `model` (rows are keyed by workspace, subagent and
  model, so one subagent on two models is two rows; `null` on a row from before the field existed),
  `calls`, `tokens`, `agents`, the usage split `input`, `cacheWrite5m`, `cacheWrite1h`, `cacheRead`,
  `output`, `thinking`, and `costUSD` with `costIsFloor`; the object itself has
  `costUSD`/`costIsFloor` too. `costUSD` is `null` when nothing was priced, and the usage split is
  `null` for a row with no per-model split) and the Markdown report has a `### Subagent usage`
  section (a `Model` column after `Subagent`, usage columns like the per-model table, an `Est. cost`
  column that shows a dash, "at least $X" or "$X", and `est. cost` on the totals line; `--no-cost`
  drops both) between the footnote and the attribution details. Both are omitted when empty, so
  `--strict` and reports without dispatched subagents are unchanged. The unattributed count spans
  all attributed records, not only the branch, and the Markdown line says so. `rows[]`, `calls`,
  `tokens`, `dispatched`, `workspaces` and `worktrees` count only verified roll-ups
  (`BranchBasis::DispatcherRolledUp`). Unlinked records stay in the branch total through the session
  claim; `unlinkedInTotal*` say how many (the Markdown line ends "N call(s) (T tokens) of them are
  included in this report's total via session claim", or "none of them ..."). Every `Source` must
  implement `dispatches` (no default body), so a wrapper cannot silently drop links. In the parity
  test, `without_subagent_divergence` checks the `subagents` object adds up (`calls` and `tokens`
  equal the sum of `rows`; absent in strict mode), removes it, and compares the rest, like
  `PRICE_DIVERGENT_KEYS`. `report::build` takes the source's dispatches, so the claim report is
  compared with the rollup applied. Strict mode stays raw, so every strict pair still equals the
  goldens. In claim mode the rollup deliberately changes six pairs, listed in `CLAIM_DIVERGENCES`
  (scenario, branch, issue `#6`, reason; a listed pair must still differ): `feat/x` in `subagent`,
  `subagent-renamed-branch` and `subagent-switches-branch` (calls stamped elsewhere roll up to the
  dispatcher), and `chore/x-part`, `chore/d1-data` and `chore/d2-data` (their records belong to the
  dispatcher, so claim mode finds none). A seventh, `main` in `unstamped-main` (issue `#37`), is
  there because Rust doesn't recover unstamped main-checkout calls onto the main checkout's
  current branch. `DIVERGENCES`, the raw-stamp comparison, stays empty: strict mode never recovers,
  so #37 can't show there.
- Output divergence #7 (attribution heuristics): the JSON report has a top-level `heuristics`
  object (`sessionClaim`, `worktreeRecovery`, `originRecovery`, `dispatcherRollUp`,
  `timelineForward`, `timelineCarried`, `ambiguous`, each `{calls, tokens}`, plus `isFloor`) that
  the reference lacks, and the Markdown footnote gains "attributed by heuristic: ..." (for example
  "dispatcher roll-up 2 call(s) / 120 tokens"). It is omitted when no heuristic assigned a call.
  A `captured` entry (`{calls, tokens}`; "captured hook data" in the footnote) is added only when
  hook-captured data placed a call, so output without it is unchanged.
  Claim attribution is unchanged; only this report field is new, so no `CLAIM_DIVERGENCES` pair is
  added. `without_heuristics_divergence` (and `strip_heuristics_divergence` in
  `branch-tally/tests/report.rs`) removes only that key, after checking it: strict mode must not
  carry it, the timeline counts are zero in claim mode, `isFloor` equals "ambiguous calls > 0", and
  `dispatcherRollUp` equals the #6 `subagents` `calls` and `tokens`. Every other key is still
  compared exactly. Timeline mode adds a `workBranch` object (`kind` `excludesDefault` with
  `defaultBranch` and `source`, or `custom` with `pattern`), and the footnote names the default
  branch when it did not come from `origin/HEAD` and any custom pattern; neither appears in claim
  or strict output.
- Output divergence #60 (unstamped calls): the JSON report has a top-level
  `"unstamped": {"calls", "tokens"}` object that the reference lacks, and the Markdown gains a
  footer line "N call(s) / T tokens in this repository carry no branch stamp and are not attributed
  to any branch". Both are omitted when N is 0. The count is repository-wide (taken over the full
  attributed set before selection), so it is the same for every branch's report; in `unstamped-main`
  it is 2 calls (`req_2`, `req_5`) and 2,221 tokens on `main` and on `feat/x`. Strict mode reports
  it too (it is raw). It counts records whose branch is `(none)` on the raw stamp, so it is disjoint
  from the subagent `unattributed*` count and from the ambiguous count. A ledger-only report cannot
  know it and omits it (the CLI already notes that unattributed counts are unavailable from the
  ledger). No `CLAIM_DIVERGENCES` pair is added. `without_unstamped_divergence` (and
  `strip_unstamped_divergence` in `branch-tally/tests/report.rs`) removes only that key, after
  checking it against the strict golden's `(none)` bucket: exactly equal in strict mode, at most
  that in claim mode, never a zero, and pinned to 2 / 2,221 for `unstamped-main`.
- A redacted summary (`Summary.redacted`, #9) is rendered by reading that flag only. JSON gains
  `"redacted": true` right after `mode`, omitted when false so parity with the reference holds.
  Markdown adds the segment `identifiers redacted` to the footnote (with or without cost), and
  the session table header reads `Span (UTC date)` with the stored dates printed whole instead of
  sliced as clock times. Insta snapshots in
  `crates/branch-tally-core/tests/snapshots/render__*redacted*.snap` pin all three.
- Output divergence #39 (Markdown escaping): the reference prints transcript-derived strings into
  the Markdown raw. Rust renders model ids, session ids, worktree names and the branch as code
  spans whose fence is longer than any run of backticks inside. In table cells it also escapes
  `|`. Agent, subagent and attribution names become plain text with `\`, backticks and `|`
  backslash-escaped and `&`, `<`, `>` as entities. Line breaks become spaces everywhere. A name
  with none of those characters renders as before, so the manual byte-parity runs still hold. The
  JSON report is unaffected. `render__markdown_hostile_names_*.snap` pins every character, and
  `every_table_row_keeps_its_column_count_whatever_the_names_contain` checks each table row
  splits like its header, the way cmark-gfm splits it.
- The bundled table was reseeded from LiteLLM's current rates (#5), not the reference's `PRICES`.
  Every model the goldens price (`claude-fable-5-1`, `claude-opus-5`, `claude-sonnet-4-6`) has the
  same input, output and cache-read rate in both, so every cost still matches the goldens exactly
  and `DIVERGENCES` stays empty. A listed divergence must still differ, so none was added. If a
  later reseed moves one of those rates, list exactly the cost keys (`totalCostUSD`, `costUSD`)
  in `DIVERGENCES` and in `crates/branch-tally/tests/report.rs`, with the reason "bundled prices
  come from LiteLLM, not token_report.py's PRICES", and keep every token and call key compared.
- The price table is tested end to end in `crates/branch-tally/tests/prices.rs`: override, cache
  and bundled lookup, the unpriced-model warning, and `prices update` and `prices show`. No test
  reaches the network: `update` talks to a loopback server through `BRANCH_TALLY_PRICES_URL`, a
  **test-only** variable. The harness points `LOCALAPPDATA` and `XDG_CACHE_HOME` (and `HOME`,
  which macOS derives the cache from) at the throwaway home so a developer's real cache never
  leaks into a run. The update logic is also unit-tested with an injected feed, date and path.
- `rust_report_matches_the_reference_claim_goldens` is live (not ignored). It compares the full
  `render::json` report with every `claim.json` key (`EXPECTED_CLAIM_PAIRS`, 35) and every
  `strict.json` key, as described above.
- The binary is checked end to end in `crates/branch-tally/tests/report.rs`: `--json` against
  the same `claim.json` and `strict.json` goldens, plus insta snapshots of the Markdown. Markdown
  byte-parity with the reference was checked **manually** (every claim branch of every
  non-divergent scenario, default, `--no-cost` and `--strict`: 81 runs, 0 differences, on a
  throwaway repo with `HOME` and `USERPROFILE` pointed at the throwaway home, as the golden
  generator does). It is not automated because the Rust test jobs have no `uv`; a test that
  skipped itself without `uv` would be a false green.

Anything that drives the script, such as the golden-output
generator, lives in `scripts/`, never in `reference/`.

## Snapshots

Rendered output (Markdown tables, JSON) is checked with snapshot tests. The proposed crate is
`insta`, confirmed in #2. Snapshots are the easiest place to fake a green:

- CI runs with snapshot updates disabled, so a changed snapshot fails.
- A new or changed snapshot is **reviewed line by line** before it is accepted. Accepting
  everything wholesale (`cargo insta accept`) turns red into green without anyone reading it.
- Snapshot diffs belong in the PR. A reviewer should see the output change.

## Docs drift

`crates/branch-tally/tests/docs_drift.rs` fails when the manual falls behind the code. It walks
the real command tree by running the binary's `--help` recursively (hidden args and subcommands
never appear there, so they are exempt) and requires every subcommand path and long flag to occur
verbatim in `docs/manual/src/cli.md`. It requires every key in `DEFAULT_CONFIG` to occur in
`docs/manual/src/config.md`. Clap's built-in `--help`, `--version` and the `help` subcommand are
skipped. The failure names each missing item and the page to update. Nothing is hand-listed, so
the test needs no upkeep. Install methods are not covered and stay a review responsibility.

## Related pages

- [project_layout.md](project_layout.md#tests): where each kind of test lives
- [transcript_format.md](transcript_format.md): the input the fixtures reproduce
- [dev_workflow.md](dev_workflow.md#quality-gates): where tests sit among the gates
