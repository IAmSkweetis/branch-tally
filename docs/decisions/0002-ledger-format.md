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

- **One usage record per line**, keyed by the API call's `requestId` and `recorded_for`, the
  branch `record` was run for. In claim mode one request can count toward two branches, so it
  appears once per branch. Each line also stores the `mode` it was selected under, and
  `report --from-ledger` keeps exactly the lines recorded for the requested branch.
- **`merge=union`** is set for the file in `.gitattributes`, so git keeps both sides' lines when
  two branches append.
- **Dedupe on read.** A reader collapses lines that share a key, so a duplicate left by a
  union merge, or by recording twice, is harmless. Writing is idempotent too: `record` appends only
  keys the ledger doesn't already hold.
- **Only what the report needs.** Usage numbers, model, timestamp, branch, session and agent ids,
  plus what the report needs to render the same tables as a live run: the attribution basis (how
  certain the branch is), the selection that kept the record, the skill and agent names, the
  subagent tags, and the session's origin. A worktree's transcript slug encodes its full path, so
  the ledger stores only the part after the main checkout's slug, and a reader puts the local
  prefix back. A worktree outside the main checkout is stored without a name, and counted. Each
  line also carries `recorded_at`, the time it was written. Never message content, never `cwd`,
  and never a path or anything that encodes one.
- **Redaction.** Every ledger write passes through a redaction step. Until #9 lands it is the
  identity; #9 supplies the policy.

## Why JSONL

- **It merges.** Union merge of line-oriented text combines concurrent appends cleanly. A SQLite
  file is a binary that conflicts on every concurrent write.
- **Duplicates cost nothing.** Keying on `requestId` and branch and deduping on read turns the
  union merge's only failure mode into a no-op.
- **It is reviewable.** A ledger change shows up in the PR diff as added lines, so a reviewer can
  see that no content was written.
- **No new dependency.** `serde_json` is already in the workspace.

## Consequences

- The file only grows. Size is bounded by the number of API calls, a few hundred bytes each.
- A union merge can't detect two lines with the same key and different data. The reader
  counts the disagreement, never hides it, and keeps the line with the latest `recorded_at`. A tie
  or a missing `recorded_at` falls back to the line that sorts first, so the choice never depends
  on line order. Lines that differ only in `recorded_at` are duplicates, not conflicts.
- Other backends stay possible. The service container (later) can import the JSONL into SQLite,
  and a different backend can sit behind the same ledger API if a need appears. Neither needs this
  decision reopened.
