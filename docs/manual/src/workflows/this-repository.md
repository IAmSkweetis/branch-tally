# This repository

branch-tally is built with Claude Code, and every pull request in its repository reports its own
token cost with branch-tally. This page describes that workflow as the repository runs it today.
The procedures live in the repository's Claude Code skills (`.claude/skills/feature/`,
`.claude/skills/create-gh-pr/` and `.claude/skills/worktree-cleanup/`), its `justfile` and its
[pull request template](https://github.com/IAmSkweetis/branch-tally/blob/main/.github/PULL_REQUEST_TEMPLATE.md).

Both [Claude Code plugins](../integration/claude-code.md) run in the repository, and the
[ledger](../ledger.md) is committed with each pull request. A PR's token report is published from
that committed ledger, not straight from the transcripts.

## The shape of the work

One Claude Code session, the **orchestrator**, runs in the repository's main checkout. It does
not write the feature itself. It:

1. interviews the user about the issue, then cuts a short-lived feature branch named per
   Conventional Branch (for example `feat/issue-4-report-command`). If the hooks have appended
   lines to `.tally/` since the last pull request, it commits them on the new branch. It then
   merges `origin/main` into the branch and runs the test suite for a baseline;
2. dispatches specialized subagents, each into its own git worktree under `.claude/worktrees/`.
   The repository sets Claude Code's `worktree.baseRef` to `head`, so each worktree starts from the
   orchestrator's current commit;
3. each subagent cuts its own branch in its worktree, commits there, and never pushes. It hands
   back a structured report naming its branch and commits;
4. merges each subagent's branch into the feature branch with `git merge --no-ff` as it lands, and
   runs the test suite after every merge;
5. hands the merged branch to the user to try on real transcripts (dogfooding), and leaves every
   worktree in place until the pull request has merged.

When the pull request is opened, the branch gains one more commit: the ledger lines that record
what the branch cost (see [Publishing the report](#publishing-the-report)).

```mermaid
gitGraph
    commit id: "start"
    branch feature
    checkout feature
    commit id: "carried .tally/ lines"
    branch agent-core
    checkout agent-core
    commit id: "core: red test"
    commit id: "core: green"
    checkout feature
    branch agent-cli
    checkout agent-cli
    commit id: "cli: red test"
    commit id: "cli: green"
    checkout feature
    merge agent-core
    merge agent-cli
    commit id: "record token usage" type: HIGHLIGHT
    checkout main
    commit id: "squash merge of the PR" type: HIGHLIGHT
```

In the diagram, `feature` is the feature branch and `agent-core` and `agent-cli` are two
subagents' branches. Two agents working on disjoint parts run at the same time. The first commit
on `feature` appears only when there were `.tally/` lines to carry over. The `record token usage`
commit adds the branch's ledger lines. The last commit on `main` stands for the squash merge: the
pull request lands as **one new commit** whose message is the PR title, and none of the branch's
own commits become part of `main`'s history. The ledger lines arrive on `main` inside it.

## The hooks and the ledger

The repository's `.claude/settings.json` declares this repository as the `branch-tally` plugin
marketplace and enables both plugins, `branch-tally-capture` and `branch-tally-record`, in the way
[shown on the Claude Code page](../integration/claude-code.md#for-everyone-who-works-in-a-repository).
Every session started in the repository therefore runs them:

```mermaid
flowchart LR
    H["Stop, SubagentStop,<br/>SessionStart"] --> C[".tally/captures.jsonl"]
    E[SessionEnd] --> L[".tally/ledger.jsonl"]
    C --> G["committed with<br/>the PR"]
    L --> G
```

- The capture hooks append to `.tally/captures.jsonl` in the main checkout, including captures
  from sessions in a worktree. See [Hook captures](../captures.md).
- When the orchestrator's session ends, the record hook appends the usage of the branch the main
  checkout has checked out to `.tally/ledger.jsonl` there.

Both files are committed. Committing them is also what keeps the repository
[opted in](../ledger.md#opt-the-repository-in): the hooks write only into a `.tally/` directory
that already exists, and because `.tally/` is in every commit, the main checkout and every
subagent's worktree have one. The repository's `.gitattributes` gives both files `merge=union`, so
lines that two branches each appended merge without a conflict:

```text
.tally/ledger.jsonl merge=union
.tally/captures.jsonl merge=union
```

The hooks write the files but never commit them, and they write on whatever branch is checked
out. So the main checkout often has uncommitted `.tally/` lines, and the skills plan for that:
their clean-tree checks ignore `.tally/` (`git status --short -- . ':!.tally'`), and the lines are
committed on the next feature branch, never on `main` and never discarded.

## Why the numbers come out right

The subagents' transcripts are written under the main checkout's transcript directory, inside the
orchestrator's session, and stamped with the orchestrator's branch or with a throwaway
`worktree-agent-…` branch. They are never stamped with the branch the subagent cut. In the default
claim mode, branch-tally:

- assigns each subagent's calls to the branch the orchestrator was on when it dispatched that
  subagent (*dispatcher roll-up*);
- resolves `worktree-agent-…` stamps through `git worktree list` (*worktree recovery*), which only
  works while the worktree still exists;
- places a session's own calls that carry no usable stamp from the [hook captures](../captures.md),
  when captures on both sides of the call agree on the branch.

That is why the usage is recorded before any worktree is removed. `record` attributes each call
when it runs and stores the branch, and how that branch was decided, in the ledger line. A report
from the ledger keeps those answers after the worktrees and transcripts are gone. Its footnote
lists the heuristics used, with the calls and tokens each one assigned.

## Publishing the report

When the work is merged into the feature branch and the full gate (`just ci`) is green, the user
runs `/create-gh-pr`. That skill:

1. checks the branch is not `main`, `gh` is signed in, the tree is clean apart from `.tally/`, and
   `main` has not moved;
2. pushes the feature branch (only the feature branch, never a subagent's);
3. opens the pull request with `gh pr create`. The body starts from the PR template, whose last
   section is:

   ```markdown
   ## Token usage

   <!-- token-report:start -->
   _Not yet recorded._
   <!-- token-report:end -->
   ```

4. records the branch's usage, commits the ledger and publishes the report from it, straight away,
   while every worktree still exists:

   ```sh
   just token-record                    # branch-tally record
   git add .tally/ledger.jsonl          # and .tally/captures.jsonl, if it changed
   git commit -m "chore: record token usage for this branch"
   git push
   just token-report --pr --from-ledger # branch-tally report --pr --from-ledger
   ```

   Both recipes run the **installed release binary**, not a build of the branch under review, so
   the number comes from a published release. When `record` appends nothing, the skill skips the
   commit and push. `report --pr --from-ledger` finds the branch's open PR and replaces the
   placeholder between the markers with the table.

The skill reads the total as printed. It says "at least $X" when some calls could not be
attributed with certainty or a model has no price. A report from the ledger also prints
`note: unattributed counts unavailable from the ledger` on stderr, because the ledger holds only
this branch's calls, so the report can't include the line counting the repository's
[unstamped calls](../cli.md#unstamped-calls). A ledger that is missing lines can understate the
total without the "at least" label, so an unlabelled number is not presented as complete.

`record` must run on the machine that did the work, within Claude Code's transcript retention
period, from sessions whose working directory was this repository.

```mermaid
sequenceDiagram
    actor User
    participant O as Orchestrator session
    participant A as Subagents (worktrees)
    participant BT as branch-tally
    participant GH as GitHub

    User->>O: /feature
    O->>O: cut feature branch, commit .tally/, merge origin/main
    O->>A: dispatch into .claude/worktrees/
    A->>A: branch, test-first, commit (no push)
    A-->>O: report: branch and commits
    O->>O: git merge --no-ff each branch, run tests
    User->>O: /create-gh-pr
    O->>GH: git push feature branch
    O->>GH: gh pr create (template with token-report markers)
    O->>BT: just token-record
    BT->>O: append to .tally/ledger.jsonl
    O->>GH: commit .tally/, git push
    O->>BT: just token-report --pr --from-ledger
    BT->>GH: gh pr edit: table between the markers
    User->>GH: squash merge
    User->>O: /worktree-cleanup
    O->>O: remove worktrees and branches
```

## Merging and cleaning up

The pull request is squash-merged, the only merge method the repository allows. Its title is a
Conventional Commit and becomes the commit message on `main`. The branch's ledger lines land on
`main` with it, so `main`'s ledger holds every merged branch.

Only after the merge does the user run `/worktree-cleanup`. It refuses to start until the PR is
merged, and it checks the PR body: if the text between the `token-report` markers still says
`_Not yet recorded._`, it asks whether to run the report **before** removing anything. Because a
squash merge leaves none of a branch's commits in `main`'s history, it decides whether each branch
landed by comparing content, not ancestry, and only then removes the worktrees and branches.

Finally it checks out `main` and fast-forwards it to `origin/main`. If uncommitted `.tally/` lines
stop the fast-forward, it leaves `main` behind and the lines uncommitted, and says so. The next
`/feature` commits them on its new branch and merges `origin/main` there.

## The project chart

The README's cost-over-time chart, `docs/assets/project-cost.svg`, comes from the committed
ledger. `just readme-graph` regenerates it with the installed release binary:

```sh
branch-tally project --export-graph docs/assets/project-cost.svg
```

It is run by hand and the result committed. The chart covers what was recorded: a branch whose
lines never reached `main`'s ledger is not in it. See [Project Cost Over Time](../project.md).
