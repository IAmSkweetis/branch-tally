---
name: worktree-cleanup
description: Remove the git worktrees and branches a piece of work left behind, after its pull request has merged. Use for /worktree-cleanup, "clean up the worktrees", "remove the agent branches", or similar.
compatibility: Requires a git repository. The merged-PR check needs the gh CLI authenticated.
metadata:
  author: branch-tally
---

# Worktree cleanup

Closing out: remove the agent worktrees under `.claude/worktrees/`, delete the local and remote
branches the work left behind, and return the main checkout to a clean `main`.

This skill deletes things. Two of those deletions can't be undone: a worktree's **uncommitted**
files, and a branch whose content isn't anywhere else. Everything below is arranged so neither
happens by accident.

## User Input

```text
$ARGUMENTS
```

## Step 0: Establish whose work this is

**If this session did the work** (it ran `/feature` and merged the agents' branches), you know the
branches and worktree paths. Use them, and still run every check in Step 2.

**If it didn't**, or you're inferring rather than remembering, **validate with the user before
changing anything.** Inventory first, present the candidates, and get explicit confirmation of each
worktree and branch. A branch you can't account for is reported and left alone. Another session of
the user's may be working in it right now.

## Step 1: Preconditions

```bash
gh pr view <number> --json state,mergedAt,headRefName,body
```

- **The PR must be merged.** If it's open, stop. The branches and worktrees are still the work.
- **The token report must already be in the PR body.** Until #6 and #13 land, attributing an agent's
  usage depends on `git worktree list` still seeing its worktree. If the body between the
  `token-report` markers still says `_Not yet recorded._`, say so and ask whether to run the report
  **before** removing anything.

If there is no PR for this work, ask the user to confirm it has landed. Don't infer it from
`git log`.

## Step 2: Inventory before you touch anything

Merges are squashed, so a merged branch's commits are never ancestors of `main`. `git branch
--merged` and `main..<branch>` will say "not landed" for work that landed cleanly. Use them as a
first pass only. The real check is **content**.

```bash
git fetch --prune
git worktree list --porcelain
git branch --list --format='%(refname:short) %(upstream:short)'
git ls-remote --heads origin
```

For each candidate worktree:

```bash
git -C <worktree-path> status --porcelain     # uncommitted or untracked work
git -C <worktree-path> branch --show-current  # the branch it holds
git diff --stat origin/main <branch>          # content not yet on main
```

These findings change the plan:

- **Uncommitted or untracked files.** Removing the worktree deletes them. List them and ask. Never
  `--force` quietly.
- **A non-empty diff against `origin/main`.** Content that didn't land. Show it and ask before
  deleting the branch. This is the one deletion with no recovery.
- **A locked worktree.** Claude Code locks a worktree while its agent runs. A held lock may mean the
  agent is still working. Check, then ask.

A large `target/` in a worktree is expected and not a reason to hesitate. It is build output.

## Step 3: Show the plan and confirm

```text
Worktrees to remove:
  .claude/worktrees/<name>        (branch <branch>, clean, content on main)
Local branches to delete:
  <agent-branch>                  (content on main)
  <feature-branch>                (merged via PR #N)
Remote branches to delete:
  origin/<feature-branch>
Leaving alone:
  <branch>                        (<why>)
```

Confirm with `AskUserQuestion`: `Remove them` (recommended), `Remove a subset`, `Cancel`.

## Step 4: Remove the worktrees

Worktrees go first, because a branch checked out in a worktree can't be deleted.

```bash
git worktree remove <path>
```

If it refuses because of uncommitted or untracked files, that's the safety net working. Go back to
the user with what it found. Use `--force` only after they've seen the list and said to discard it.

On Windows, a long-running build or an editor holding a file in `target/` can make removal fail
with a permission error. Find the process (kill only one you started), or ask the user to close it.
Don't retry in a loop.

## Step 5: Delete the branches

**Local**, one at a time:

```bash
git branch -d <branch>
```

After a squash merge, `-d` refusing is **expected**: it decides by ancestry. If Step 2's content
diff was empty, use `-D`. If it wasn't, or you skipped Step 2, stop and go back. Use `-D` only
after the user has seen what is being discarded, and never in a loop.

**Remote.** Agents don't push, so normally only the feature branch exists remotely:

```bash
git ls-remote --heads origin <feature-branch>
git push origin --delete <feature-branch>     # only if it's still there
```

Never delete `main`, locally or remotely. Never force-push.

## Step 6: Tidy and verify

```bash
git worktree prune
git checkout main
git pull --ff-only
git worktree list
git branch -a
git status --short
```

Report what was removed, what was left alone and why, and the state `main` is in. Offer `just test`
if the user wants confidence the checkout is usable. Report the exit code **and the test count**.

## What this skill never does

- Remove a worktree or delete a branch **before** the PR has merged.
- Remove a worktree before the token report has been recorded.
- Use `--force` or `-D` without the user having seen exactly what is being discarded.
- Delete `main`, force-push, or touch a branch it can't account for.
- Touch `~/.claude/projects/`. Transcripts are the user's data, not cleanup debris.
