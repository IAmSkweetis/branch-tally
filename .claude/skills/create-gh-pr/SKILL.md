---
name: create-gh-pr
description: Open a GitHub PR for the current branch with a Conventional Commit title, the right label, its issue linked, added to the branch-tally project, and its Claude Code token usage recorded in the body. Use for /create-gh-pr, "open a PR", "create a pull request", or similar.
compatibility: Requires a GitHub remote, the gh CLI authenticated (with the project scope for the board step), and a branch with commits
metadata:
  author: branch-tally
---

# Create GitHub PR

Open a pull request for the current branch, give it the label its branch type implies, link its
issue, put it on the project board, and record what the work cost in tokens before the transcripts
expire.

This project measures its own PRs. That's dogfooding, and it's also how we find out where the tool
is wrong.

## User Input

```text
$ARGUMENTS
```

## Step 0: Prerequisites

- `git branch --show-current` must not be `main`. If it is, stop and ask.
- `gh auth status` must be authenticated.
- `git status --short` should be clean. If there is uncommitted work, ask before going on.
- `just ci` should be green. Record the exit code **and the test count**. If it's red, say so and
  ask whether to open the PR as a draft.
- `git fetch origin && git merge-base --is-ancestor origin/main HEAD`: if `main` has moved, offer
  to merge it in first (trunk-based: integrate before review).
- `git push -u origin <branch>` once the user agrees to open the PR. The `pre-push` hook runs
  `just test` and checks the branch name. If it fails, fix what it found.
- The PR title and branch are checked again in CI (`pr-title`, `branch-name`, `commit-messages`),
  so a title that isn't a Conventional Commit turns the PR red.

Only this branch is pushed, never an agent's worktree branch.

## Step 1: Derive the kind label from the branch

| Branch prefix | Label |
| ------------- | ----- |
| `feat/`, `feature/` | `enhancement` |
| `fix/`, `bugfix/`, `hotfix/` | `bug` |
| `chore/` with only `docs:` commits | `documentation` |
| `chore/` otherwise, `release/` | none: this repository has no matching label |

Check with `git log --oneline origin/main..HEAD`. If the branch matches no prefix, ask with
`AskUserQuestion`. Don't guess and don't rename the branch.

Never create a label without being asked. If none fits, open the PR unlabelled rather than
substituting a wrong one. Also apply `roadmap` when the linked issue carries it.

## Step 2: Issue link

If the branch name embeds an issue number (`feat/issue-4-…`), confirm it with `gh issue view <N>`.
Use `Fixes #N` when the PR completes the issue's "Done when" checklist, and `Refs #N` when it only
advances it. Never invent a link. If there is no issue, ask whether one should exist. Roadmap work
always has one.

## Step 3: Compose

Title: a Conventional Commit matching the branch type (`feat(attribution): …`, `fix: …`,
`docs: …`). It becomes the squash commit on `main` and drives the release version, so get the type
and any `!` right.

Body: start from `.github/PULL_REQUEST_TEMPLATE.md`, the single source for the sections and the
`token-report` markers. Read it at run time rather than copying the text below, which only shows
its shape. Fill in each section and leave the markers exactly as they are, because Step 5 replaces
what sits between them.

```markdown
## Summary

- 1–3 bullets on **why**. The diff already says what.

## Related

Fixes #N

## Test plan

- [ ] `just ci` is green, and the test count looks right (not zero, and up by the tests added)
- [ ] A red run was observed before the green, for each behaviour change
- [ ] Dogfooded on real local transcripts (if the change affects output)
- [ ] Snapshot changes reviewed line by line (if any)

## Notes for reviewers

## Token usage

<!-- token-report:start -->
_Not yet recorded._
<!-- token-report:end -->

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

Ask the user which test-plan boxes are genuinely true. Don't tick them by default. Show the title,
body and labels, and confirm with `AskUserQuestion`: `Create it` (recommended), `Edit a section`,
`Cancel`.

## Step 4: Create it

Write the body to a temp file outside the repository and pass `--body-file`. Inlining the body
mangles backticks and the markers.

```bash
gh pr create --title "<title>" --body-file <path> --label "<kind>" [--label roadmap]
```

Don't pass `--project` to `gh pr create`. A missing token scope would fail the whole command. Add
the PR to the board separately:

```bash
gh project item-add 2 --owner IAmSkweetis --url <pr-url>
```

If that fails with a scope error, the PR is fine. Tell the user to run
`gh auth refresh -s project` and retry the `item-add`. Don't run the refresh yourself.

## Step 5: Token usage

Run this immediately once the PR exists, **while every agent worktree still exists**:

```bash
just token-report --pr      # runs the installed `branch-tally report --pr`
```

It uses the **installed release binary**, not `cargo run` on this branch. If `branch-tally` isn't
on `PATH`, the recipe stops with an install hint. Tell the user to install the latest
[release](https://github.com/IAmSkweetis/branch-tally/releases/latest) archive, and leave the
placeholder until they have. Don't install it yourself, and don't fall back to
`reference/token_report.py`: it is only the goldens' parity oracle now. Don't paste a number from
another tool or machine.

When reading the output:

- **It must run on the machine that did the work**, within `cleanupPeriodDays` (default 30).
- **The session's cwd must have been this repository.** Work started from another directory is
  filed under that directory's transcripts and is invisible here.
- **It's a floor.** Until #6 and #7 land, worktree subagents and multi-branch sessions are
  under-counted. Say "at least $X". Where the number looks wrong to the user, that's a finding:
  offer to open an issue for it.

If the script fails, report what it said and move on. A missing table is worth reporting, never
worth failing the PR over.

## Notes

- This skill doesn't request reviewers, doesn't merge, and doesn't run cleanup. The user merges
  (squash only), then runs `/worktree-cleanup`.
- Never force-push to update a PR. Push new commits.
