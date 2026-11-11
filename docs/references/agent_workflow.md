# Agent workflow

How work gets handed to a specialized agent, kept out of the way of other agents running at the
same time, and handed back. [agents.md](agents.md) is the roster: who exists and what each one
writes. This page is the process around them.

It sits on top of [dev_workflow.md](dev_workflow.md). Everything there applies to an agent exactly
as it applies to a person: trunk-based branches, Conventional Commits, the quality gates.

## When to use subagents and worktrees

Use them when the work is **complex**: it spans several modules, it has independent parts that can
proceed in parallel, or it is large enough that one context would fill up with file dumps. A
roadmap issue that touches the model, attribution and the CLI is the typical case.

Don't use them for a one-line fix in a file you're already looking at, a docs tweak, or anything
you'd finish in a handful of tool calls. A dispatch costs a worktree, a build, a branch and a
review. See [When an agent is the wrong tool](#when-an-agent-is-the-wrong-tool).

## Why worktrees

Several agents can run at once, each in its own git worktree, so their branches merge cleanly. That
only holds while they touch **disjoint files**. That is why the roster is cut along filesystem
lines, and why every dispatch names a slice.

A Rust workspace has its own collision hotspots:

| File | Why it hurts |
| ---- | ------------ |
| `Cargo.toml`, `Cargo.lock` | Every new dependency touches them. Two clean-merging additions can still change feature unification or re-resolve the lockfile. |
| `crates/branch-tally-core/src/lib.rs` | Every new module is declared here. |
| `crates/branch-tally-core/src/model/` | The shared types. A field added on one branch breaks constructors on another, and the merge looks clean. |

The first two belong to the Workspace Config Steward, and `lib.rs` and `model/` to whichever Core
Implementer holds the `model` slice. Everyone else escalates.

## Shared resources, not just shared files

Worktrees are disjoint on disk. They are not disjoint everywhere else:

- **`~/.claude/projects/` is live data.** Agents dogfooding the CLI read the user's real
  transcripts, including the transcript of the session that dispatched them. **Never write, move
  or delete anything there.** Run the tool read-only, and never copy a raw line into the repository
  (Governance III).
- **The cargo build directory.** Each worktree has its own `target/` by default. Leave it that way.
  Pointing parallel agents at one shared `CARGO_TARGET_DIR` serializes their builds on cargo's file
  lock, and one agent's `cargo clean` wipes the others' builds. The cost is a slow first build per
  worktree. `worktree-bootstrap` starts it early.
- **Processes.** Kill only the PID you started. Never kill by image name
  (`taskkill /IM cargo.exe`, `pkill rustc`): that takes out every other agent's build and the
  user's own.
- **The git stash.** It is shared by every worktree of the repository, so another session can pop
  yours. Use a WIP commit instead.

## Starting an agent

A whole piece of work starts with the **`feature` skill** (`/feature`). It interviews the user,
cuts the branch in the main checkout, dispatches the agents, merges their branches back as they
land, and hands off to dogfooding and `create-gh-pr`. This section describes what that skill is
doing. Read it when dispatching a single agent by hand, or when the skill hits a decision it can't
make for you.

1. **Create the worktree** with `isolation: "worktree"` on the Agent call. The agent runs
   `worktree-bootstrap` first.
2. **Name the branch** per [dev_workflow.md](dev_workflow.md): `<type>/<description>`, named for
   the change rather than the agent.
3. **Name the slice.** For a Core Implementer or a Harness Source Author this is required:

   > Core Implementer: slice = `attribution`
   >
   > Harness Source Author: harness = `claude_code`

   Two agents on different slices never collide. Two on the same slice always do. An agent given
   no slice must ask for one before writing anything.

### Where a worktree starts

`.claude/settings.json` sets `worktree.baseRef` to `head`. A subagent dispatched with
`isolation: "worktree"` therefore branches from the **dispatching session's `HEAD`**, which lets an
agent build on work already merged into the feature branch. The default, `fresh`, would hand every
agent a checkout of `main` without it.

Two consequences:

- The orchestrator must **commit before every dispatch**. An uncommitted change in the main
  checkout never reaches the worktree.
- A session sitting on `main` produces worktrees based on `main`. That's right for a one-off agent
  and wrong mid-feature.

## Dispatch by slice, not by layer

Splitting "write the tests" from "write the code" puts two agents in the same files and breaks
red→green. Each implementing agent owns both halves. Cuts that look tidy on paper ("someone who
writes tests", "someone who does the parser") put two writers in one directory. Cuts that follow
the tree don't.

Dispatch everything independent in **one message with several Agent calls**. Sequence only what
genuinely depends on earlier work. A model change comes first, in its own wave. A CLI flag that
needs a core accessor nobody has written yet goes in the second wave.

## Handing work back

Every agent ends with a report per
[`.claude/agent-report-contract.md`](../../.claude/agent-report-contract.md): one fenced JSON block,
the last thing in its final message.

The parts that matter when reading one:

- **`verification` is an array**, so a test-first agent reports its red run *and* its green run.
  That makes "did test-first actually happen" checkable rather than assumed.
- **`tests_run` is required on every `cargo_test` entry.** `exit_code: 0` with `tests_run: 0` is a
  false green, and requiring both numbers turns that trap into a detectable lie.
- **`dogfood` entries carry evidence**: the command run and the numbers it printed, with no
  transcript content.
- **`complete_no_changes` is a success.** An auditor that found nothing wrong did its job.
- **`blocked` and `failed` must each carry an escalation.**
- **`user_facing_changes` feeds the Docs Author.** When any agent in a wave lists one, dispatch the
  Docs Author in the last wave with those entries, after the behaviour has merged.

## Escalation routing

An agent that needs something outside its scope raises an escalation instead of reaching for it.

| Situation | Escalation | Goes to |
| --------- | ---------- | ------- |
| A new or changed field on `Record` or another `model` type | `model_change_required` | Core Implementer holding `model` |
| The CLI or an integration needs something core doesn't expose | `needs_core_api` | Core Implementer for that module |
| A test needs a transcript situation no fixture reproduces | `fixture_required` | Harness Source Author |
| A new crate, feature flag, version bump or toolchain change | `dependency_request` | Workspace Config Steward |
| A change to a hook, workflow, recipe, skill, or lint/runner config | `file_outside_scope` | Repository Tooling Steward |
| Real transcript content, a home path or a secret is about to be committed or printed | `privacy_risk` | stop; back to the orchestrator |
| Implementation requested with no failing test | `missing_red_test` | back to the requester |
| A pipeline stage reached past its neighbour (a source deciding branches, a renderer computing totals) | `boundary_violation` | whoever owns the file |
| A number would be reported with more certainty than the data supports | `overstated_claim` | whoever owns the attribution or render code |

A `dependency_request` is **reviewed, not rubber-stamped**. The Steward checks size, maintenance,
licence and whether the standard library or an existing dependency already does the job.

## Landing the work

An agent's branch goes through the same gates as anyone's. The report's `branch` and
`worktree_path` let the orchestrator merge it without guessing.

A linked worktree shares the main checkout's object store and refs, so a local commit is durable
the moment it exists. An agent **commits before finishing and does not push**. The orchestrator
merges the agent's branch into the feature branch and pushes that one branch. Pushing every agent
branch would only leave dead remote branches behind.

The exception is work handed off across sessions, where no orchestrator in the same session will do
the merge. There the agent pushes. The dispatch must say so explicitly; it is never the default.

**Uncommitted** work in a worktree is lost when the worktree is removed. The commit is the
durability boundary, not the push.

## Closing out

Order matters, and the reason is this project's own subject:

1. The feature branch is merged, green, and dogfooded.
2. **`create-gh-pr`** pushes the branch, opens the PR, and records the token usage **while every
   agent worktree still exists**.
3. The PR merges.
4. **`worktree-cleanup`** removes the worktrees and branches, and returns the main checkout to a
   clean `main`.

Run step 4 early and, until #6 and #13 land, the token report can't attribute those agents' usage.
The reference script maps `worktree-agent-<id>` records onto branches by asking `git worktree list`.
Fixing that dependency is one of this project's first jobs. Until it's fixed, we live with it.

## When an agent is the wrong tool

A dispatch costs a worktree, a fresh `target/` build, a branch and a review. For a small change in
a file you're already looking at, just make the change, test-first, on a short-lived branch. The
roster earns its keep on work large enough to need isolation, or parallel enough to need several
agents at once.

## Related pages

- [agents.md](agents.md): the roster and write-scope matrix
- [dev_workflow.md](dev_workflow.md): branches, commits, PRs, quality gates
- [testing.md](testing.md): what a trustworthy red and green look like
