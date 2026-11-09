# Project layout

Where everything lives and, more usefully, why it lives there. The tree itself is the authority.
This page explains the distinctions the tree can't, which are mostly about **which of two plausible
places a new file belongs in**.

> **Status:** the Cargo workspace (#3) and the fixtures (#2) exist. Rows marked
> *planned* describe the agreed target. Whoever lands the change updates this page in the same
> commit, and drops the marker once the path is real.

## Top level

| Path | Holds | Notes |
| ---- | ----- | ----- |
| `crates/branch-tally-core/` | The library: every piece of logic | See below. |
| `crates/branch-tally/` | The binary: CLI parsing and wiring | Thin by design. `src/prices.rs` holds the `prices` subcommand, the `ureq` `PriceFeed`, the cache path and the table lookup `report` shares (#5). |
| `tests/fixtures/<harness>/` | Scrubbed transcript trees, one directory per scenario | Shared by both crates' tests. See [testing.md](testing.md#fixtures). |
| `reference/token_report.py` | The original Python script, **immutable** | The parity oracle, run only by the golden generator (`just goldens`). This repository's PR token reports come from branch-tally itself (#41). It is never edited, not even to fix a bug, and nothing else is ever added to `reference/`. New Python goes in `scripts/`. |
| `data/` | Data shipped with the binary, such as the bundled price table | The bundled price table lives in `crates/branch-tally-core/data/prices.toml` so the crate packages it (#5). Tuned, not coded. |
| `plugin/` | The **shipped** Claude Code plugin: manifest and the skill users install | *planned* (#11). Not this repository's own skills. |
| `action.yml` | The **shipped** GitHub Action | *planned* (#12). Must sit at the root for `uses: IAmSkweetis/branch-tally@v1`. |
| `docs/references/` | Reference pages | This page lives here. |
| `docs/decisions/` | Architecture decision records, `NNNN-title.md` | Append-only. Supersede; don't rewrite. |
| `scripts/` | Repository tooling that is not a `just` one-liner: the commit-subject and branch-name checks (shared by the hooks and CI), their self-test, CI helpers, release packaging and smoke test (`package.sh`, `smoke-test.sh`), and **every Python script** the project writes (such as the fixture scrubber and golden generator, #2) | Run through `just`, a hook or a workflow. Python runs with `uv`, never a system interpreter. |
| `.githooks/` | Git hooks (`pre-commit`, `commit-msg`, `pre-push`), enabled with `core.hooksPath .githooks` | See [dev_workflow.md](dev_workflow.md#quality-gates). |
| `.github/` | CI, PR-check, release (`release.yml`, `build.yml`) and workflow-audit workflows, PR template, `CODEOWNERS`, `dependabot.yml`, `zizmor.yml` | CI for this repository, not the shipped Action. See [dev_workflow.md](dev_workflow.md#quality-gates). |
| `.claude/` | This repository's **own** agents, skills, settings and report contract | Development tooling. Not shipped. |
| `Cargo.toml` | Workspace manifest: members, `[workspace.dependencies]`, `[workspace.lints]`, profiles | Lints are declared once here and each crate opts in with `lints.workspace = true`. |
| `Cargo.lock` | Committed: this is a binary project | |
| `rust-toolchain.toml` | Pinned toolchain and components | Pinned at 1.99.1, matching `rust-version` ([0001](../decisions/0001-language.md)). |
| `rustfmt.toml`, `clippy.toml`, `.config/nextest.toml` | Formatter, linter and test-runner config | |
| `deny.toml` | Dependency policy | Licences, advisories, bans and sources. CI's `deny` job runs it. |
| `justfile` | The command entrypoint, with the pinned tool versions | See [dev_workflow.md](dev_workflow.md#commands). |
| `.gitattributes`, `.gitignore`, `.rumdl.toml` | Formatting, line endings, ignores, Markdown lint | |
| `README.md`, `CONTRIBUTING.md`, `LICENSE` | What the project is and its links; workspace setup and how a change lands; MIT | `CONTRIBUTING.md` summarises [dev_workflow.md](dev_workflow.md). Change both together. |

## Shipped vs. development: the distinction that gets confused

Three things in this repository look alike and are not:

| | Shipped to users | This repository's own tooling |
| - | ---------------- | ----------------------------- |
| Claude Code skill | `plugin/skills/…` | `.claude/skills/…` |
| GitHub automation | `action.yml` (an Action other repositories run) | `.github/workflows/…` (CI for this repository) |
| Python | none | `scripts/*.py` (tooling), and `reference/token_report.py` (immutable test oracle) |

A shipped skill or Action is **product code**. It is versioned with releases, documented for
strangers, tested, and must work on a machine that has never seen this repository. A development
skill or workflow serves only us. They have different owners
([agents.md](agents.md#who-owns-a-given-path)), and one must never be "fixed" by editing the other.

## Inside `branch-tally-core` *(`model` and the `Source` trait exist; `git`, `attribution`, `summary`, `pricing`, `render`, `report`, `pr` are implemented; others planned)*

The modules follow the pipeline, in order. A module may depend only on modules **above** it in this
table, plus `model`:

| Module | Holds | Must not |
| ------ | ----- | -------- |
| `model` | `Record`, `Usage`, ids: the normalized types every stage shares | contain logic beyond constructors and accessors |
| `sources/` | The `Source` trait and its shared types (`sources/mod.rs`); each harness lives in `sources/<harness>/` (`claude_code/` exists; others *planned*). Discovering a harness's transcript roots for a repo and parsing them into `Record`s | decide which branch a record belongs to |
| `git` | The trait for asking git about branches and worktrees, with the real implementation | be called directly by anything that should take the trait |
| `attribution` | Assigning each record to a branch, with a certainty level | format output, or read files itself |
| `summary` | Aggregating attributed records into the by-model, by-attribution and by-session tables (the reference's `summarize`) | assign branches, price, or format output |
| `ledger` | Reading and writing the repository ledger (#8) | know about any one harness |
| `pricing` | Costing a summary from a `PriceTable`. Rates live in data, never in Rust: the bundled `crates/branch-tally-core/data/prices.toml` (embedded with `include_str!`, schema in its header), a per-user cache and `.tally/prices.toml`. Lookup is override, then cache, then bundled; each is a complete table and the first found wins whole, so one table has one date. A malformed override or cache is an error naming the file, never a fall-through. `from_litellm` converts the LiteLLM feed behind the `PriceFeed` trait (the HTTP impl is the CLI's) | define a price in code, merge tables per model, or fetch over the network |
| `render` | Markdown and JSON output (CSV is deferred to #36) from an already-computed summary | compute totals or read transcripts |
| `report` | The pipeline entry point: `build` (pure: records + worktrees + branch + strict to `Summary` and `Costs`, or an explicit no-records error) and `run` (the reference's `main()` resolution behind the `Git` and `Source` traits) | attribute, price or format anything itself; it only chains the stages |
| `pr` | The `Gh` trait (the GitHub CLI, with a fake for tests) and `update_pr`: find the PR, splice the report into its body, say what happened. The real `SystemGh` writes the body to gh as UTF-8 bytes | build the report, or run anywhere but the CLI's `--pr` path |
| `redact` | Applying the redaction policy (#9) | run anywhere except just before output or ledger write |

Should a new file go in core or the CLI? **Ask whether a test could call it without a terminal.**
If it could, it belongs in core. The CLI crate parses arguments, resolves the repository and
current branch, calls core, writes to stdout, and talks to `gh`. It holds no logic worth unit
testing on its own. Its tests are end-to-end: run the binary against a fixture and compare the
output.

The CLI's output is a public contract: stdout carries only the report (Markdown or JSON, UTF-8,
LF), byte-identical to the reference. Honest-numbers notes go to stderr as single `note:` lines
with counts only: `git worktree list` unavailable (worktree recovery skipped), and records the
read could not use (malformed lines, records without a request id, synthetic records). Replays
removed by de-duplication are not announced, since resumed sessions make them routine. The notes
also print before a no-records error.

## Tests

Rust has three places a test can live, and each has a use:

| Kind | Where | Use it for |
| ---- | ----- | ---------- |
| Unit | `#[cfg(test)] mod tests` at the bottom of the file under test | One function or type, including private items |
| Integration | `crates/<crate>/tests/<module>.rs`, one file per core module | The module's public API against fixtures |
| End-to-end | `crates/branch-tally/tests/` | Running the binary: output, exit codes, the PR splice (with `gh` faked; until a fake `gh` executable is portable, the splice is tested in core against `FakeGh`) |
| Doc | `///` examples on public items | Keeping documented examples true. **Not run by nextest.** |

A test file outside a crate's `tests/` directory, or not declared as a module, is **never
compiled**, and nothing tells you. `just test`'s count is how you notice.

## Related pages

- [testing.md](testing.md): how the tests are written and what makes one trustworthy
- [agents.md](agents.md): who writes which of these paths
