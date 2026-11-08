# Agents

The specialized agents in `.claude/agents/`: who exists, and what each one is allowed to write.

This page is the roster. [agent_workflow.md](agent_workflow.md) covers the process around it:
dispatching, worktrees, reporting and escalation routing.

## How the roster is cut

Agents are divided along **filesystem** lines rather than conceptual ones. Parallel worktrees merge
cleanly only when their branches touch disjoint files. A few files would be fought over by
everyone, so each has a single owner who is serialized against itself.

There is deliberately **no separate test-writing agent.** Splitting the failing test from the code
that makes it pass puts two agents in the same files and breaks the red→green loop. Every
implementing agent owns both halves.

## Parallel-safe: run several at once, one slice each

| Agent | Writes | Verified by |
| ----- | ------ | ----------- |
| **Core Implementer** | one named module of `crates/branch-tally-core/src/` (not `sources/`), its unit tests, and `crates/branch-tally-core/tests/<module>.rs`. The `pricing` slice also owns `data/**`. | red + green `cargo_test` |
| **Harness Source Author** | one harness: `crates/branch-tally-core/src/sources/<harness>/`, `crates/branch-tally-core/tests/sources_<harness>.rs`, `tests/fixtures/<harness>/**` | red + green `cargo_test`, plus a scrub check on every fixture |
| **CLI Builder** | `crates/branch-tally/src/**`, `crates/branch-tally/tests/**` | red + green `cargo_test`, plus `dogfood` evidence |
| **Integration Author** | `plugin/**`, `action.yml`, `action/**`, `examples/**`: the surfaces users install into their harness or CI | red + green test where one is possible, plus `dogfood` evidence |

### The `model` slice is special

`crates/branch-tally-core/src/lib.rs` and `src/model/` are read by every other module. Only one
Core Implementer holds the `model` slice at a time, and new modules are declared in `lib.rs` by the
agent holding it. An agent that needs a new field on `Record` raises `model_change_required`. It
doesn't add the field itself. The orchestrator lands model changes first, in their own wave, so
later agents branch from a `HEAD` that already has them.

## Serialized: never two of the same one at once

| Agent | Writes |
| ----- | ------ |
| **Workspace Config Steward** | `Cargo.toml` (root and every `crates/*/Cargo.toml`), `Cargo.lock`, `rust-toolchain.toml`, `deny.toml`, and release-tool config (`dist-workspace.toml`, `release-plz.toml`, `cliff.toml`, or whichever #10 settles on) |
| **Repository Tooling Steward** | `justfile`, `scripts/**`, `.githooks/**`, `.github/**`, `.claude/skills/**`, `.claude/agent-report-contract.md`, `rustfmt.toml`, `clippy.toml`, `.config/nextest.toml`, `.rumdl.toml`, `.gitattributes`, `.gitignore`, `.gitleaksignore`, `reference/**` (frozen) |

Each one is serialized **against itself**, never against the other. Their files never meet, so the
two can run in parallel. The split follows how each kind of file fails:

- **Manifests** fail silently. Two branches that each add a dependency merge cleanly and still
  change what gets built: feature unification, a resolver bump, a lockfile that re-resolves. Every
  dependency is also a supply-chain decision for a tool that reads private transcripts.
- **Automation** fails loudly. A bad merge to a hook or workflow breaks the next time it runs.

Neither owns `.claude/agents/**` or `.claude/settings.json`. An agent able to edit the file that
defines its own scope, or the file that grants tool permissions, could grant itself anything, and
every boundary here would mean nothing. A human changes the roster and the permissions, or the
orchestrator does when a human directs it. The change lands before the work that needs it.

## Read-only: always safe alongside anything

| Agent | Writes |
| ----- | ------ |
| **Code and Docs Auditor** | nothing |

## Who owns a given path

| Path | Owner |
| ---- | ----- |
| `crates/branch-tally-core/src/<module>/` (except `sources/`, `model/`) | Core Implementer, one module per agent |
| `crates/branch-tally-core/src/lib.rs`, `src/model/` | Core Implementer holding the `model` slice, one at a time |
| `crates/branch-tally-core/src/sources/<harness>/` | Harness Source Author, one harness per agent |
| `crates/branch-tally-core/tests/<module>.rs` | whoever owns that module |
| `tests/fixtures/<harness>/**` | Harness Source Author for that harness |
| `crates/branch-tally/**` (except `Cargo.toml`) | CLI Builder |
| `plugin/**`, `action.yml`, `action/**`, `examples/**` | Integration Author |
| `data/**` | Core Implementer holding the `pricing` slice |
| every `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `deny.toml`, release-tool config | Workspace Config Steward |
| `justfile`, `scripts/**`, `.githooks/**`, `.github/**`, `.claude/skills/**`, lint, format and runner config | Repository Tooling Steward |
| `reference/token_report.py` | Repository Tooling Steward, frozen: edited only to fix a bug also fixed in Rust |
| `.claude/agents/**`, `.claude/settings.json` | nobody: a scope boundary an agent can edit is not a boundary |
| `docs/references/**` | whichever agent changed what the page describes, in the same commit |
| `docs/decisions/**` | the orchestrator, recording a decision the user made |
| `AGENTS.md` | the orchestrator, at the user's direction |

## Skills agents rely on

| Skill | Covers |
| ----- | ------ |
| `worktree-bootstrap` | Making a fresh worktree's test results trustworthy. Run it first, always. |
| `rust-tests` | Running the suite, the false-green traps, snapshots and fixtures. |

And three that only the **orchestrator** runs, never an agent:

| Skill | Covers |
| ----- | ------ |
| `feature` | Starting a piece of work: the interview, the branch, dispatching this roster into worktrees, merging each branch back, and the handoff to dogfooding and the PR. |
| `create-gh-pr` | Pushing the one branch, opening the PR with its label, issue link and project, and recording its token usage. |
| `worktree-cleanup` | Removing worktrees and branches after the PR has merged, never before. |

## Deliberately not in the roster

- **A test-writing agent.** It breaks red→green and forces two agents into one slice.
- **A docs agent.** Whoever changes a thing updates its reference page, in the same commit.
- **A release agent.** Releases are cut by CI from `main` ([release.md](release.md)). What remains
  is configuration, which belongs to the Workspace Config Steward and the Repository Tooling
  Steward.
- **A per-harness agent for every harness.** There is one Harness Source Author definition. A
  dispatch names which harness it owns this time.

## Related pages

- [agent_workflow.md](agent_workflow.md): dispatch, worktrees, reporting, escalations
- [dev_workflow.md](dev_workflow.md): branches, commits, PRs, quality gates
- [project_layout.md](project_layout.md): what each path is for
