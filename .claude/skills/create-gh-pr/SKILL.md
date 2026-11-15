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
expire. The usage is recorded and committed **before the first push**, so the PR opens with its
table already in the body and nothing is pushed or edited afterwards just to add it.

This project measures its own PRs. That's dogfooding, and it's also how we find out where the tool
is wrong.

## User Input

```text
$ARGUMENTS
```

## Step 0: Prerequisites

- `git branch --show-current` must not be `main`. If it is, stop and ask.
- `gh auth status` must be authenticated.
- `git status --short -- . ':!.tally'` should print nothing. If there is other uncommitted work, ask
  before going on. Step 3 commits `.tally/`, so changes there are expected.
- `just ci` should be green. Record the exit code **and the test count**. If it's red, say so and
  ask whether to open the PR as a draft.
- `git fetch origin && git merge-base --is-ancestor origin/main HEAD`: if `main` has moved, offer
  to merge it in first (trunk-based: integrate before review).
- Don't push yet. The first push comes in Step 5, after the token usage is committed.
- The PR title and branch are checked again in CI (`pr-title`, `branch-name`, `commit-messages`),
  so a title that isn't a Conventional Commit turns the PR red.

Only this branch is pushed, never an agent's worktree branch.

## Step 1: Derive the kind label from the commit type

The label follows the commit type of the PR title (Step 4), which the branch prefix narrows down:

| Commit type | Label |
| ----------- | ----- |
| `feat` | `enhancement` |
| `fix` | `bug` |
| `docs` | `documentation` |
| `build`, `ci`, `test`, `perf`, `refactor` | the label of the same name |
| `chore`, `style` | `chore` |

A `feat/` or `feature/` branch is `feat`, and a `fix/`, `bugfix/` or `hotfix/` branch is `fix`. A
`chore/` branch can be any of the other types: read `git log --oneline origin/main..HEAD`, and when
its commits mix types, use the type the title will carry. If the branch matches no prefix, or the
type is unclear, ask with `AskUserQuestion`. Don't guess and don't rename the branch. A `release/`
branch gets no label.

Never create a label without being asked. If none fits, open the PR unlabelled rather than
substituting a wrong one. When the linked issue has a milestone, give the PR the same one.

## Step 2: Issue link

If the branch name embeds an issue number (`feat/issue-4-…`), confirm it with `gh issue view <N>`.
Use `Fixes #N` when the PR completes the issue's "Done when" checklist, and `Refs #N` when it only
advances it. Never invent a link. If there is no issue, ask whether one should exist. Roadmap work
always has one. If not, write `No issue: <reason>`.

## Step 3: Token usage, before the first push

Run this **before** the branch is pushed and while every agent worktree still exists. It needs no
PR: `record` attributes by branch, and `report --from-ledger` reads the ledger.

```bash
just token-record                    # runs the installed `branch-tally record`
git add .tally/ledger.jsonl          # and .tally/captures.jsonl too, if it exists
git commit -m "chore: record token usage for this branch"
just token-report --from-ledger --md > <tmp>/report.md   # the table, bracketed by the token-report markers
```

Only add the `.tally/` files that changed. If `record` changed nothing, skip the commit. The commit
must be a valid Conventional Commit (`chore:`), because the `commit-messages` check runs on it.
`<tmp>` is outside the repository. Step 4 splices `report.md` into the body.

Why this order: every push to an open PR starts the checks again and dismisses any approval
(`dismiss_stale_reviews_on_push`). A reviewer that skips a ledger-only commit (as CodeRabbit does,
see [dev_workflow.md](../../../docs/references/dev_workflow.md#code-review)) never approves again,
so the PR could not merge without an admin bypass. A trailing `.tally/`-only push, or a body edit to
fill the table, is exactly that. So no push of this PR is ever just token usage.

It uses the **installed release binary**, not `cargo run` on this branch. If `branch-tally` isn't
on `PATH`, the recipes stop with an install hint. Tell the user to install the latest
[release](https://github.com/IAmSkweetis/branch-tally/releases/latest) archive, and leave the
placeholder until they have. Don't install it yourself, and don't fall back to
`reference/token_report.py`: it is only the goldens' parity oracle now. Don't paste a number from
another tool or machine.

When reading the output:

- **It must run on the machine that did the work**, within `cleanupPeriodDays` (default 30).
- **The session's cwd must have been this repository.** Work started from another directory is
  filed under that directory's transcripts and is invisible here.
- **Read the label as printed.** The total says "at least $X" only when some calls can't be
  attributed with certainty or a model is unpriced. A ledger missing entries can understate the
  total without that label, so don't present an unlabelled number as complete. `report
  --from-ledger` also prints `note: unattributed counts unavailable from the ledger`, because the
  ledger holds only this branch's calls (see the manual's
  [ledger chapter](../../../docs/manual/src/ledger.md)).
  Where the number looks wrong to the user, that's a finding: offer to open an issue for it.
- **It stops at this commit.** Work after it (review fixes, any later round) is not in the table
  until that round records its own usage (see "Later rounds" below). Add this line under the
  table in the body, outside the markers so a later `report --pr` doesn't remove it:
  `_Recorded before review: usage from later rounds is not included, so this is a floor._`

If `record` or `report` fails, or prints `No ledger records found` (no markers in its output),
report what it said, leave the `_Not yet recorded._` placeholder as it is, **omit the floor
line** (there is no table for it to qualify), and go on. Tell the user why the table is missing
(no ledger records for the branch, a missing binary, or the error printed). A missing table is
worth reporting, never worth failing the PR over.

## Step 4: Compose

Title: a Conventional Commit matching the branch type (`feat(attribution): …`, `fix: …`,
`docs: …`). It becomes the squash commit on `main` and drives the release version, so get the type
and any `!` right.

Body: start from `.github/PULL_REQUEST_TEMPLATE.md`, the single source for the sections and the
`token-report` markers. Read it at run time rather than copying the text below, which only shows
its shape. Fill in each section and leave the markers exactly as they are, because Step 3's report
replaces what sits between them.

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

Once the user confirms, splice the table into the body file, so the PR opens with it:

```bash
uv run scripts/splice_token_report.py <body-file> <tmp>/report.md   # replaces the placeholder
```

It exits non-zero and leaves the body untouched when either file lacks the markers.

## Step 5: Push and create it

Push the branch only now, once the user agrees to open the PR. The `pre-push` hook runs
`just test` and checks the branch name. If it fails, fix what it found.

```bash
git push -u origin <branch>
```

Write the body to a temp file outside the repository and pass `--body-file`. Inlining the body
mangles backticks and the markers.

```bash
gh pr create --title "<title>" --body-file <path> --label "<kind>" [--milestone v1]
```

Don't pass `--project` to `gh pr create`. A missing token scope would fail the whole command. Add
the PR to the board separately:

```bash
gh project item-add 2 --owner IAmSkweetis --url <pr-url>
```

If that fails with a scope error, the PR is fine. Tell the user to run
`gh auth refresh -s project` and retry the `item-add`. Don't run the refresh yourself.

## Later rounds

Review fixes, a merge of a moved `main`, or any other work after the PR opens: record that round's
usage and commit it **together with the round's work**, in the same push, never as its own push
afterwards.

```bash
just token-record
git add .tally/ledger.jsonl           # and .tally/captures.jsonl too, when it changed
git commit -m "chore: record token usage for this branch"  # if it changed
git push                              # one push: the fixes and the ledger together
just token-report --pr --from-ledger  # optional: refreshes the table, which edits the body
```

A body edit doesn't cancel checks (the workflows ignore body-only edits), and the reviewer's next
review covers a head that already includes the ledger. The last round's own usage is then missing
from the table, which is why the table carries the floor line from Step 3. Don't add a
`.tally/`-only push after the reviewer has approved.

## Telling a reviewer's real state from a skipped check

A reviewer's check on the head SHA reads `skipping` when it skipped that commit's files
(CodeRabbit excludes `.tally/**`, for one). That says nothing about its review. Read the reviews,
not the check:

```bash
gh pr view --json reviewDecision,headRefOid,reviews \
  --jq '{decision: .reviewDecision, head: .headRefOid, reviews: [.reviews[] | {who: .author.login, state, commit: .commit.oid}]}'
```

- `decision` of `APPROVED` means the requirement is met now. `REVIEW_REQUIRED` means there is no
  current approval: look at the latest review per author.
- A review whose `commit` is not `head` was on an earlier commit. A push dismisses an approval
  there, so the PR is waiting for a new one. If every commit since is `.tally/`-only and the
  reviewer skips them, it never comes: the deadlock this skill avoids by recording before the
  push. Report it. Don't bypass the ruleset yourself.
- `CHANGES_REQUESTED` or open threads: the reviewer is waiting on a fix, not on a re-run.

## Notes

- This skill doesn't request reviewers, doesn't merge, and doesn't run cleanup. The reviewer
  configured for this repository reviews non-draft PRs on its own, and its approval is the one
  `main` requires (today that is CodeRabbit, see
  [dev_workflow.md](../../../docs/references/dev_workflow.md#code-review)). For a draft PR, tell the
  user it gets no review until CI is fixed and it is marked ready for review. Otherwise, end by
  telling the user the PR is waiting on review, and that a review-fixing tool such as
  `/coderabbit:autofix` (if installed) works through unresolved threads. Record that round's token
  usage in the same push as its fixes (Later rounds).
  The user merges (squash only), then runs `/worktree-cleanup`.
- Never force-push to update a PR. Push new commits.
