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
| `crates/branch-tally/` | The binary: CLI parsing and wiring | Thin by design. `src/config.rs` supplies core's `ConfigEnv` (real env and fs) and renders `config show` and `config init` (which writes core's `DEFAULT_CONFIG` template); `report` resolves its flags through core's `config` module (flag > file > default, #28). `src/prices.rs` holds the `prices` subcommand, the `ureq` `PriceFeed`, the cache path and the table lookup `report` shares (#5). `src/report.rs` holds `report` and `record` (#8): `record` appends the branch's new records to `.tally/ledger.jsonl` at the root of the current checkout (`Git::toplevel` of the `--repo` path or cwd, so a subdirectory or linked worktree writes at its own root; outside a repository it is an error, never a fallback to cwd; write only, never `git add` or commit) only when `.tally/` already exists as a real directory: otherwise it writes and creates nothing, prints one stable note and exits 0, and a symlink or non-regular file at `.tally` or `.tally/ledger.jsonl` is an error naming it (#94; the checkout root above `.tally` is trusted, not checked). `report --from-ledger [--with-live]` and `project` read it back through the same guard (`report::ledger_target` picks core's `LedgerTarget::Default`); `--ledger <path>` is exempt and overrides the file for all of them, so tests and dogfooding never touch a real ledger. The `.tally/prices.toml` override is read through core's `TallyDir`; `capture` writes nothing, silently, when the main checkout has no `.tally/`, and refuses a symlink with a fixed one-line message. `src/update.rs` holds the `update` subcommand (alias `upgrade`): the `ureq` or `file://` `ReleaseClient`, the extracting and replacing `Installer`, the stdin `Confirmer`; every decision it makes is core's `update` module. `src/capture.rs` holds the `capture` subcommand (#13): it reads a hook payload from stdin, builds a `CaptureEvent` (stamping `ts` from the ledger `SystemClock`) and appends it to `.tally/captures.jsonl` through core. |
| `tests/fixtures/<harness>/` | Scrubbed transcript trees, one directory per scenario | Shared by both crates' tests. See [testing.md](testing.md#fixtures). |
| `reference/token_report.py` | The original Python script, **immutable** | The parity oracle, run only by the golden generator (`just goldens`). This repository's PR token reports come from branch-tally itself (#41). It is never edited, not even to fix a bug, and nothing else is ever added to `reference/`. New Python goes in `scripts/`. |
| `data/` | Data shipped with the binary, such as the bundled price table | The bundled price table lives in `crates/branch-tally-core/data/prices.toml` so the crate packages it (#5). Tuned, not coded. |
| `plugin/` | The **shipped** Claude Code plugins: `capture-hooks/` and `record-hooks/` (each with `.claude-plugin/plugin.json` and `hooks/hooks.json`), `README.md` and `tests/check.sh`. The skill (`skills/`) is *planned* (#11) | Not this repository's own skills. |
| `.claude-plugin/marketplace.json` | The marketplace manifest listing the `plugin/` plugins | Must sit at the repository root. |
| `action.yml` | The **shipped** GitHub Action | *planned* (#12). Must sit at the root for `uses: IAmSkweetis/branch-tally@v1`. |
| `install.sh`, `install.ps1` | The **shipped** one-line installers, also attached to each release as assets (#46) | POSIX sh and PowerShell. Tested by `tests/install/`. Only tools a stock OS has; no `gh`. |
| `tests/install/` | `install_test.sh` and `install_test.ps1`: run the installers against a fake `file://` release | Plain scripts, no framework; each prints its case count. |
| `docs/manual/` | The **user manual**: `book.toml` and the mdBook pages in `src/`, published to GitHub Pages | For users, not contributors. Documents what ships on `main` ([0003](../decisions/0003-user-manual.md)). |
| `docs/manual/src/images/` | Images the manual embeds: `branch-tally-logo.svg`, the project logo, also shown at the top of `README.md` | Traced in Inkscape from a GIMP drawing that is not committed, then reduced to its visible paths, cropped to 600 px wide, with a light outline under the dark lines so the lettering reads on GitHub's dark theme. Kept under `src/` because mdBook only publishes files there, so the README points here rather than keeping a second copy in `docs/assets/`. |
| `docs/references/` | Reference pages | This page lives here. |
| `docs/assets/` | Images the README embeds: `project-cost.svg`, this repository's cost-over-time chart, and `pr-example.png`, a screenshot of a PR token report | `project-cost.svg` is generated by `just readme-graph`, never hand-edited. `pr-example.png` is a hand-taken screenshot, scaled to 800 px wide and reduced to a 128-colour palette. Both show branch names, ids, dates and numbers only, never transcript content. |
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
| Claude Code skill | `plugin/skills/…` (*planned*, #11) | `.claude/skills/…` |
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
| `model` | `Record`, `Usage`, ids: the normalized types every stage shares, and `CaptureEvent`/`CaptureKind` (`model/capture.rs`, a hook-reported branch fact, #13), and the project-report types `ProjectReport`/`BranchRow`/`ProjectTotals`/`SharedOverlap`/`SeriesPoint` (`model/project.rs`, #87; rows keyed by `recorded_for`, totals deduplicated by request id, floor flags only), `model/timestamp.rs` (#95), `Timestamp`: the one strict RFC 3339 type (`YYYY-MM-DDTHH:MM:SS`, optional fraction, `Z` or `±HH:MM`, fields range-checked) carried by `Record.ts`, `LedgerEntry.ts` and `BranchRow.last_call`. It keeps the text as read, orders by that text (session first/last stay raw-string ordered, as the reference does), and exposes `instant()` for ordering across offsets. It is built once where a record or ledger line is read; text that is not a timestamp becomes `None` and is counted (`ReadStats::malformed_timestamps`, `ledger::ReadStats::malformed_timestamps`), never carried. `Dispatch.ts` and `CaptureEvent.ts` stay `String`: they only order attribution, through `model/instant.rs`'s `parse_ts`, a thin wrapper over the same parser, and reach no output | contain logic beyond constructors and accessors |
| `sources/` | The `Source` trait and its shared types (`sources/mod.rs`); each harness lives in `sources/<harness>/` (`claude_code/` exists; others *planned*). Discovering a harness's transcript roots for a repo and parsing them into `Record`s. `claude_code/hook.rs` parses the hook's stdin payload (`HookPayload`, #13) | decide which branch a record belongs to |
| `text` | Shared text utilities for output stages. `replace_control_chars` swaps C0, C1, DEL and the Unicode bidi marks, overrides and isolates for U+FFFD; borrowed when nothing needs replacing (#95) | do I/O, or know about Markdown or terminals |
| `paths` | Path-text normalisation (verbatim and UNC prefixes) and transcript slugs, shared by `git`, `sources` and `attribution`; no filesystem access | do I/O, or be re-implemented by a caller |
| `git` | The trait for asking git about branches and worktrees, with the real implementation. Methods: `current_branch`, `toplevel` (the root of the checkout containing a dir, so a linked worktree's own root; typed error outside a repo), `main_checkout`, `worktrees`, `default_branch`, `init_default_branch`, `file_at` (a repo-relative path's bytes at a rev; `Ok(None)` only when the path is absent, an unknown rev is an error) | be called directly by anything that should take the trait |
| `attribution` | Assigning each record to a branch, with a certainty level | format output, or read files itself |
| `summary` | Aggregating attributed records into the by-model, by-attribution and by-session tables (the reference's `summarize`), plus `footnote.heuristics` (calls and tokens per heuristic over the selected records; `ambiguous` comes from `count_ambiguous` over the full attributed set; left at default in strict, the raw view) | assign branches, price, or format output |
| `project` | Ledger to `ProjectReport` (#87): per-`recorded_for` rows, request-id-deduplicated totals, overlap, cumulative series, all by parsed timestamp instant; a branch with no parseable timestamp is undated and counted, and calls with a missing or unparsable timestamp are counted per row and project-wide (`undated_calls`) because they can leave `last_call` early. Pure: no git, clock or filesystem | format output, read transcripts |
| `ledger` | The repository ledger, append-only JSONL at `.tally/ledger.jsonl` ([decision 0002](../decisions/0002-ledger-format.md)). `LedgerEntry` is one line (versioned, keyed by `(request_id, recorded_for)`: `recorded_for` is the branch `record` ran for and `mode` the selection mode, so a request two branches claim is kept once per branch; usage, model, ts, the assigned `branch`, session/agent ids, basis, `kept_by`, skill/agent names, subagent tags, the session `origin` (`StoredOrigin`: `"main"`, `{"worktree": "<name>"}` or `{"worktree": null}`; the name is the transcript slug minus the main checkout's `paths::slug` prefix, matched case-insensitively with leading separators trimmed, e.g. `claude-worktrees-agent-abc`, because a full slug encodes the user's path; a slug outside the main checkout is stored unnamed and counted in `AppendOutcome::unnamed_origins`; `to_selected`, `union` and `append` take the main checkout path, and a read re-prefixes the name with its current slug, so an unnamed origin reads as unknown; absent on older lines, which read as none), and `recorded_at`, the RFC 3339 UTC time `append` stamped from its `Clock`; never content or cwd; `is_sidechain` is not stored and `to_selected` derives it as `agent_id.is_some()`) with `from_selected(selected, recorded_for, mode)`/`to_selected`. `Ledger::for_branch` filters on `recorded_for` and `Ledger::modes` lists the modes present. `read` treats a missing file as empty, collapses duplicate keys and counts duplicates, conflicts (same key, different data including mode, once `recorded_at` is ignored: the latest `recorded_at` wins, a missing one is oldest, a tie takes the smallest serialized line), malformed and unsupported-version lines in `ReadStats`. `append(path, records, recorded_for, mode, ..)` writes only keys not yet held, through the `Redact` seam (`Identity` until #9) and stamps `recorded_at` from an injected `Clock` (`SystemClock` is the real one), and returns `AppendOutcome`. `union` merges a ledger with live records and counts each side (the ledger's copy wins). The file needs `merge=union` in `.gitattributes`. `ledger/captures.rs` does the IO for `.tally/captures.jsonl` (`append_captures` and the read with its stats, #13). `ledger/tally_dir.rs` is the guarded accessor for everything core reads or writes under the default `.tally/` (#94): `TallyDir::open(root)` is `Ok(None)` when `.tally` does not exist (the repository has not opted in: nothing is created, and writes report `tally_absent` on their outcome), refuses a symlink or non-directory `.tally` or one that does not canonicalize to a direct child of `root`, and holds a handle on `.tally` for its lifetime: on Unix a directory fd (`rustix`, `O_DIRECTORY \| O_NOFOLLOW`), and every file is opened with `openat` relative to it, so swapping `.tally` for a link afterwards changes nothing; on Windows a backup-semantics, open-reparse-point handle shared without `FILE_SHARE_DELETE`, so `.tally` cannot be renamed or replaced while it is held. `read`/`append` take a single plain file name (no separator, `.`, `..`, NUL or `:`), check the target without following links, open it no-follow and re-check the handle is a regular file; a refusal is `LedgerError::Refused`, never an empty read. The checkout root is the trust anchor; the module docs state the limits. `ledger/check.rs` is the pure check behind `ledger check --base <ref>` (#96): `check(&CheckInput) -> CheckReport` takes the base and head bytes of both files (each optional), the head branch and `now`, and lists `Violation { file, line, rule }` (never line text): a base line missing from the head (multiset), a new line that does not parse, and for new ledger lines a `recorded_for` other than the head branch, a missing, invalid or future `recorded_at`, a `(request_id, recorded_for)` key already on base, or a token field over `MAX_TOKENS_PER_FIELD` (10,000,000); new capture lines must parse and carry no future `ts` (no branch rule, #115). `read_captures`, `append_captures`, `read_default` and `append_default` use it, and no core function creates `.tally/`. An explicit `--ledger <path>` is exempt: `read` and `append` take the path as is (`LedgerTarget::Default` or `Explicit` picks between them in `report::record_at` and `run_from_ledger_at`; the default ledger root is `Git::toplevel`, so a linked worktree keeps its own ledger, while captures and `prices.toml` use the main checkout). The binary reads `.tally/prices.toml` through `TallyDir::read("prices.toml")` (`prices.rs`, `GuardedText`) and prints the refusal text as is | know about any one harness, store content or cwd, or implement a redaction policy |
| `config` | The user config file: `mode`, `work-branch`, `format`, `cost`, `redact` (default off), `redact-names` (default on) defaults, merged flag > file > default, each value tagged with its source. Reads env and files only through the `ConfigEnv` trait (the real impl is the CLI's). `work-branch` stays an uncompiled string | compile regexes, look up a home dir itself, or accept unknown keys silently |
| `pricing` | Costing a summary from a `PriceTable`. Rates live in data, never in Rust: the bundled `crates/branch-tally-core/data/prices.toml` (embedded with `include_str!`, schema in its header), a per-user cache and `.tally/prices.toml`. Lookup is override, then cache, then bundled; each is a complete table and the first found wins whole, so one table has one date. A malformed override or cache is an error naming the file, never a fall-through. `from_litellm` converts the LiteLLM feed behind the `PriceFeed` trait (the HTTP impl is the CLI's) | define a price in code, merge tables per model, or fetch over the network |
| `render` | Markdown and JSON output (CSV is deferred to #36) from an already-computed summary; `render::project` formats a `ProjectReport` as Markdown with a Mermaid chart, JSON and a standalone SVG (#87) | compute totals or read transcripts |
| `report` | The pipeline entry point: `build` (pure: records + worktrees + branch + `Options` to `Summary` and `Costs`, or an explicit no-records error) and `run` (the reference's `main()` resolution behind the `Git` and `Source` traits). `Options` picks `Mode` (strict, claim, timeline) and, for timeline, the work-branch rule: the default (all but the default branch and `worktree-agent-*`; `run` resolves the default branch from `origin/HEAD`, then `init.defaultBranch`, then assumes `main`, and records which on `Summary.work_branch`) or a caller-supplied predicate. `record` (#8) selects through the same step as `run` and appends the branch's records to the ledger (idempotent; returns the append outcome and the source read stats). `run_from_ledger` reports from the ledger (entries with `recorded_for` equal to the branch, exactly), optionally unioned with live transcripts (the ledger's copy wins), and carries the ledger read stats, union counts and the modes the entries were recorded under (more than one means mixed) on `Report.ledger`. A ledger-only report cannot know the repository-wide unattributed counts (`FullSetCounts::Unavailable`: unknown, not zero); a union takes them from the live side. Live reads (`run`, `record`, and the live side of `run_from_ledger`) also read `.tally/captures.jsonl` from the main checkout (#13) and attribute through `build_with_captures`, so captured branches win over worktree recovery and show as the `captured` heuristic; a missing file changes nothing, and what the read counted (bad lines included) is on `Report.capture_stats` and `Recorded.capture_stats`. A missing ledger with no live side is `ReportError::LedgerMissing` | attribute, price or format anything itself; it only chains the stages |
| `pr` | The `Gh` trait (the GitHub CLI, with a fake for tests) and `update_pr`: find the PR, splice the report into its body, say what happened. The real `SystemGh` writes the body to gh as UTF-8 bytes | build the report, or run anywhere but the CLI's `--pr` path |
| `redact` | Applying the redaction policy (#9) | run anywhere except just before output or ledger write |
| `update` | Self-update decisions for `branch-tally update`: `Version` (parse, compare, `is_dev_version`), release asset naming (`archive_name`, `current_target` over the one `TARGETS` list, `UNSUPPORTED_TARGET` sentinel), `SHA256SUMS` parsing (`verify_checksum`, `latest_version_from_sums`), `sha256_hex`, `is_binary_entry` (is this archive path the binary for a target), and `run`, which orders check, confirm, download, verify and install behind the `ReleaseClient`, `Installer` and `Confirmer` traits | touch the network, the filesystem or the terminal; those are the CLI traits' impls in `crates/branch-tally/src/update.rs` |

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

A ledger-backed report adds notes of the same shape: `unattributed counts unavailable from the
ledger` (ledger only; the repository-wide counts are unknown, not zero), `ledger mixes modes (…)`,
`skipped N ledger line(s)` (malformed, unsupported version) and a count of conflicting lines
resolved. Duplicate lines are not announced, for the same reason as replays. `--with-live` also
prints the live read's notes. `record` adds one: `N record(s) from a worktree outside the main
checkout were stored without a worktree name` (the ledger stores worktree origins relative to the
main checkout, never a path), printed only when N is above zero.

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
