# 0002. Store the ledger as append-only JSON Lines

- **Status:** accepted
- **Date:** 2026-11-10
- **Issue:** #8

## Context

A PR's token cost comes from more places than one machine's transcripts: the local orchestrator,
subagents in worktrees, other machines and CI runs. Transcripts are also purged after Claude
Code's `cleanupPeriodDays`. branch-tally therefore keeps attributed usage in a ledger committed to
the repository, so the PR total can include every source and outlive the transcripts.

The ledger is written concurrently. Each agent worktree records on its own branch, those branches
merge into the feature branch, and the feature branch squash-merges into `main`. The format has to
survive those merges without conflicts and without counting a call twice. The open question was
JSON Lines or SQLite.

## Decision

The ledger is **append-only JSON Lines** at `.tally/ledger.jsonl`.

- **One usage record per line**, keyed by the API call's `requestId`.
- **`merge=union`** is set for the file in `.gitattributes`, so git keeps both sides' lines when
  two branches append.
- **Dedupe on read.** A reader collapses lines that share a `requestId`, so a duplicate left by a
  union merge, or by recording twice, is harmless. Writing is idempotent too: `record` appends only
  request ids the ledger doesn't already hold.
- **Only what the report needs.** Usage numbers, model, timestamp, branch, session and agent ids,
  plus what the report needs to render the same tables as a live run: the attribution basis (how
  certain the branch is), the selection that kept the record, the skill and agent names, and the
  subagent tags. Never message content, and never `cwd`.
- **Redaction.** Every ledger write passes through a redaction step. Until #9 lands it is the
  identity; #9 supplies the policy.

## Why JSONL

- **It merges.** Union merge of line-oriented text combines concurrent appends cleanly. A SQLite
  file is a binary that conflicts on every concurrent write.
- **Duplicates cost nothing.** Keying on `requestId` and deduping on read turns the union merge's
  only failure mode into a no-op.
- **It is reviewable.** A ledger change shows up in the PR diff as added lines, so a reviewer can
  see that no content was written.
- **No new dependency.** `serde_json` is already in the workspace.

## Consequences

- The file only grows. Size is bounded by the number of API calls, a few hundred bytes each.
- A union merge can't detect two lines with the same `requestId` and different data. The reader
  has to choose one deterministically and count the disagreement, never hide it.
- Other backends stay possible. The service container (later) can import the JSONL into SQLite,
  and a different backend can sit behind the same ledger API if a need appears. Neither needs this
  decision reopened.
