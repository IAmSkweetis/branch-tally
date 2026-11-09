# Project layout

Where everything lives and, more usefully, why it lives there. The tree itself is the authority.
This page explains the distinctions the tree can't, which are mostly about **which of two plausible
places a new file belongs in**.

> **Status:** the Cargo workspace does not exist yet. It is created by #3, and the reference script
> and fixtures by #2. Rows marked *planned* describe the agreed target. Whoever lands the change
> updates this page in the same commit, and drops the marker once the path is real.

## Top level

| Path | Holds | Notes |
| ---- | ----- | ----- |
| `crates/branch-tally-core/` | The library: every piece of logic | *planned* (#3). See below. |
| `crates/branch-tally/` | The binary: CLI parsing and wiring | *planned* (#3). Thin by design. |
| `tests/fixtures/<harness>/` | Scrubbed transcript trees, one directory per scenario | *planned* (#2). Shared by both crates' tests. See [testing.md](testing.md#fixtures). |
| `reference/token_report.py` | The original Python script, frozen | The parity oracle, and what `just token-report` runs until #4. Never edited except to fix a bug that is also fixed in Rust. |
| `data/` | Data shipped with the binary, such as the bundled price table | *planned* (#5). Tuned, not coded. |
| `plugin/` | The **shipped** Claude Code plugin: manifest and the skill users install | *planned* (#11). Not this repository's own skills. |
| `action.yml` | The **shipped** GitHub Action | *planned* (#12). Must sit at the root for `uses: IAmSkweetis/branch-tally@v1`. |
| `docs/references/` | Reference pages | This page lives here. |
| `docs/decisions/` | Architecture decision records, `NNNN-title.md` | Append-only. Supersede; don't rewrite. |
| `scripts/` | Repository tooling that is not a `just` one-liner: the commit-subject and branch-name checks (shared by the hooks and CI), their self-test, and CI helpers | Run through `just`, a hook or a workflow. |
| `.githooks/` | Git hooks (`pre-commit`, `commit-msg`, `pre-push`), enabled with `core.hooksPath .githooks` | See [dev_workflow.md](dev_workflow.md#quality-gates). |
| `.github/` | CI and PR-check workflows, workflow audit, PR template, `CODEOWNERS`, `dependabot.yml`, `zizmor.yml` | CI for this repository, not the shipped Action. See [dev_workflow.md](dev_workflow.md#quality-gates). |
| `.claude/` | This repository's **own** agents, skills, settings and report contract | Development tooling. Not shipped. |
| `Cargo.toml` | Workspace manifest: members, `[workspace.dependencies]`, `[workspace.lints]`, profiles | *planned* (#3). |
| `Cargo.lock` | Committed: this is a binary project | |
| `rust-toolchain.toml` | Pinned toolchain and components | *planned* (#3). |
| `rustfmt.toml`, `clippy.toml`, `.config/nextest.toml` | Formatter, linter and test-runner config | |
| `deny.toml` | Dependency policy | *planned* (#3). CI's `deny` job starts checking once it exists. |
| `justfile` | The command entrypoint, with the pinned tool versions | See [dev_workflow.md](dev_workflow.md#commands). |
| `.gitattributes`, `.gitignore`, `.rumdl.toml` | Formatting, line endings, ignores, Markdown lint | |
| `README.md`, `CONTRIBUTING.md`, `LICENSE` | What the project is and its links; workspace setup and how a change lands; MIT | `CONTRIBUTING.md` summarises [dev_workflow.md](dev_workflow.md). Change both together. |

## Shipped vs. development: the distinction that gets confused

Three things in this repository look alike and are not:

| | Shipped to users | This repository's own tooling |
| - | ---------------- | ----------------------------- |
| Claude Code skill | `plugin/skills/…` | `.claude/skills/…` |
| GitHub automation | `action.yml` (an Action other repositories run) | `.github/workflows/…` (CI for this repository) |
| Python | none | `reference/token_report.py` (test oracle only) |

A shipped skill or Action is **product code**. It is versioned with releases, documented for
strangers, tested, and must work on a machine that has never seen this repository. A development
skill or workflow serves only us. They have different owners
([agents.md](agents.md#who-owns-a-given-path)), and one must never be "fixed" by editing the other.

## Inside `branch-tally-core` *(planned, #3)*

The modules follow the pipeline, in order. A module may depend only on modules **above** it in this
table, plus `model`:

| Module | Holds | Must not |
| ------ | ----- | -------- |
| `model` | `Record`, `Usage`, ids: the normalized types every stage shares | contain logic beyond constructors and accessors |
| `sources/<harness>/` | Discovering a harness's transcript roots for a repo and parsing them into `Record`s | decide which branch a record belongs to |
| `git` | The trait for asking git about branches and worktrees, with the real implementation | be called directly by anything that should take the trait |
| `attribution` | Assigning each record to a branch, with a certainty level | format output, or read files itself |
| `ledger` | Reading and writing the repository ledger (#8) | know about any one harness |
| `pricing` | Loading price tables and costing a `Usage` | hard-code a price (#5) |
| `render` | Markdown, JSON and CSV output from an already-computed summary | compute totals or read transcripts |
| `redact` | Applying the redaction policy (#9) | run anywhere except just before output or ledger write |

Should a new file go in core or the CLI? **Ask whether a test could call it without a terminal.**
If it could, it belongs in core. The CLI crate parses arguments, resolves the repository and
current branch, calls core, writes to stdout, and talks to `gh`. It holds no logic worth unit
testing on its own. Its tests are end-to-end: run the binary against a fixture and compare the
output.

## Tests

Rust has three places a test can live, and each has a use:

| Kind | Where | Use it for |
| ---- | ----- | ---------- |
| Unit | `#[cfg(test)] mod tests` at the bottom of the file under test | One function or type, including private items |
| Integration | `crates/<crate>/tests/<module>.rs`, one file per core module | The module's public API against fixtures |
| End-to-end | `crates/branch-tally/tests/` | Running the binary: output, exit codes, the PR splice (with `gh` faked) |
| Doc | `///` examples on public items | Keeping documented examples true. **Not run by nextest.** |

A test file outside a crate's `tests/` directory, or not declared as a module, is **never
compiled**, and nothing tells you. `just test`'s count is how you notice.

## Related pages

- [testing.md](testing.md): how the tests are written and what makes one trustworthy
- [agents.md](agents.md): who writes which of these paths
