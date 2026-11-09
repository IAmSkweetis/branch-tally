---
name: feature
description: Orchestrate a piece of work from interview to merge. Collect the issue, context and guidance, cut a short-lived branch from main, dispatch specialized agents into worktrees, merge their branches back as they land, and hand off to dogfooding and the PR. Use for /feature, "start a feature", "work on issue N", "implement a feature", or similar.
compatibility: Requires a git repository with a clean main checkout. Agent worktrees require `worktree.baseRef` set to `head` in `.claude/settings.json`.
metadata:
  author: branch-tally
---

# Feature orchestration

You are the **orchestrator** for this piece of work. You run the interview, cut the branch,
dispatch specialized agents into worktrees, merge their work back, and hand off to dogfooding and
the PR. You don't implement the feature yourself. That is what the roster in
`docs/references/agents.md` is for.

Read `docs/references/agent_workflow.md` before Step 3. This skill is the procedure. That page holds
the reasoning, including which files two agents must never both touch.

**First, decide whether this needs the machinery at all.** A change you'd finish in a handful of
tool calls doesn't need worktrees. Do it directly, test-first, on a short-lived branch, and say
that's what you're doing.

## User Input

```text
$ARGUMENTS
```

## Step 1: Interview the user

Ask with `AskUserQuestion`, and **invent nothing**.

1. **Issue.** Which roadmap issue is this? Skip if `$ARGUMENTS` names one. Read it with
   `gh issue view <N>`. Its "Done when" checklist is the acceptance criteria. If there is no issue,
   ask whether to open one first. Planned work lives in issues (#16 is the tracking issue).
2. **Context.** Does it depend on, replace or extend other work? Is anything in flight that it
   must not collide with?
3. **Guidance.** Design notes, numbers that must hold, things that are off the table.

**Don't infer guidance from the title.** "Prices as data" says nothing about which feed to trust.
A guessed decision that reaches an agent becomes a decision nobody made. With no guidance, record
`None given`. That tells the agents to ask instead of assuming.

Reflect the answers back verbatim:

```markdown
| Field | Value |
| ----- | ----- |
| Issue | #N: title |
| Context | ... |
| Guidance | ... |
| Done when | (the issue's checklist) |
```

Confirm with `AskUserQuestion`: `Looks right` (recommended), `Refine it`, `Cancel`. Don't proceed
until the user has confirmed the table as written.

## Step 2: Prepare the workspace

The branch is cut in the **main checkout**, from an up-to-date `main`. Trunk-based: it will live
for hours or days, not weeks.

```bash
git status --short          # must be clean. If not, stop and ask.
git checkout main
git pull --ff-only
git checkout -b <type>/issue-<N>-<description>
```

Name it per Conventional Branch (`docs/references/dev_workflow.md#branch-names`), for the change.
If the tree is dirty, stop and ask. **Never stash.** The stash is shared across every worktree.

Establish a green baseline before anyone starts:

```bash
just test
```

Record the exit code **and the test count**. A green baseline is what makes a later red run
attributable to one specific merge. If the issue is big enough that this branch would live for
more than a few days, stop and propose splitting it.

## Step 3: Dispatch the agents

### Check the worktree base first

`worktree.baseRef` in `.claude/settings.json` must be `"head"`. If it reads `"fresh"`, stop and
tell the user. Don't work around it.

**Commit before every dispatch.** Agents branch from `HEAD`, and uncommitted changes don't reach
them.

### Split the work by slice, not by layer

| The work is | Agent | Slice to name |
| ----------- | ----- | ------------- |
| Shared types (`Record`, `Usage`), or a new module declaration | Core Implementer | `model`, alone, in the **first** wave |
| Attribution, pricing, ledger, rendering, redaction, git access | Core Implementer | one module, **required** |
| Reading a harness's transcripts, plus its fixtures | Harness Source Author | one harness, **required** |
| Flags, output wiring, `gh` integration | CLI Builder | `crates/branch-tally` |
| The shipped plugin or skill, the Action, examples | Integration Author | `plugin/` or `action.yml` |
| A dependency, the toolchain, release config | Workspace Config Steward | serialized |
| A hook, workflow, recipe, this repo's skills, lint config | Repository Tooling Steward | serialized |
| A sweep before the PR | Code and Docs Auditor | read-only, any time |

Two rules decide what runs at once:

- Parallel agents touch **disjoint paths**. Two Core Implementers on `attribution` always collide.
  On `attribution` and `pricing`, they never do.
- Each Steward is serialized **against itself** only.

Dispatch everything independent in **one message with several Agent calls**. Sequence only what
depends on earlier work: model first, then the modules that use it, then the CLI that exposes them.

### The dispatch

Every agent runs with `isolation: "worktree"`. Give each one the confirmed table plus its slice:

```text
Issue: #<N>: <title>
Context: <verbatim>
Guidance: <verbatim, or "None given; ask before assuming a value">
Done when: <the items this agent is responsible for>
Your slice: <module / harness / path>
Branch: cut your own <type>/<description> branch in your worktree, per Conventional Branch.

Run the `worktree-bootstrap` skill first.
Test-first: write the failing test, watch it fail for the intended reason, then make it pass.
Never write transcript content into the repository or your report.
Commit before you finish. Do NOT push. The orchestrator merges your branch.
Report per `.claude/agent-report-contract.md`: one fenced JSON block, the last thing in your message.
```

### Reading the report back

Parse the last fenced JSON block. Before merging:

- `status` is `complete` or `complete_no_changes`. If it's `blocked` or `failed`, route the
  escalation per `agent_workflow.md#escalation-routing` and re-dispatch. Don't fix it yourself
  inside the agent's slice.
- An implementing agent has a **red** `cargo_test` entry and a **green** one. A single green entry
  means test-first didn't happen. Send it back.
- Every `cargo_test` entry has `tests_run`. Green with `0` is a false green.
- `branch`, `worktree_path` and `commits` are populated.
- No `privacy_risk` escalation is open.

## Step 4: Merge each agent's branch back

From the main checkout, on the feature branch, as each agent lands:

```bash
git merge --no-ff <agent-branch> -m "chore: merge <slice> into <feature-branch>"
just test
```

Run the suite **after every merge**, so a red run points at one merge. On a conflict in a manifest,
`Cargo.lock`, `lib.rs` or `model/`, stop and resolve it deliberately. Never take `--ours` or
`--theirs` wholesale. Regenerate `Cargo.lock` with cargo rather than hand-merging it.

**Leave every worktree in place** until the PR has merged (Step 6).

## Step 5: Hand off to dogfooding

When everything is merged and `just ci` is green, the branch is code-complete but not yet
verified on real data. Report with numbers:

```text
#<N> is merged into <branch>. just ci is green (<N> tests, exit 0).

Merged: <agent>: <summary> (<branch>)

Ready to dogfood. Try: cargo run -p branch-tally -- <command> on this repository.
What to check: <the behaviour each agent changed, and the numbers that should move>
```

Run the binary yourself first, read-only, and include its summary line. Ask the user to compare it
with what they expect from their own sessions. Tests prove behaviour on fixtures. Dogfooding is
where real transcripts disagree with the fixtures. If they do, that's a new fixture and a new red
test, dispatched the same way.

## Step 6: What happens next (don't do these unprompted)

1. The user runs **`/create-gh-pr`**. It pushes the branch, opens the PR, and records the token
   usage while the worktrees still exist.
2. The PR is squash-merged.
3. The user runs **`/worktree-cleanup`**.

Offer each step. Don't run one on your own initiative.

## The rules that bite here

- **Never commit to `main`, never force-push, never merge without the PR.**
- **Never remove a worktree before the PR has merged.**
- **Never push an agent's branch.** Only the feature branch is pushed, by `/create-gh-pr`.
- **Never write to `~/.claude/projects/`.** Dogfooding is read-only.
- **Kill only PIDs you started.** Never kill `cargo` or `rustc` by image name.
- **Uncommitted work in a worktree is lost with the worktree.** An agent reporting no commits has
  handed back nothing.
