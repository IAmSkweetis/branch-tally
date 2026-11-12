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

## Addendum: the captures file (#13)

Hook-captured attribution events (`model::CaptureEvent`) are stored in a sibling file,
`.tally/captures.jsonl`, not in `ledger.jsonl`, so v1 ledger readers never see a line they do not
understand. The format follows the ledger's: append-only JSON Lines, one event per line in field
order, LF endings, `merge=union` in `.gitattributes`.

- **Dedupe key:** the whole event, `ts` included. A replayed hook produces an exact duplicate; a
  writer skips it and a reader collapses and counts it.
- **Ids only.** An event holds session and agent ids and resolved branch names, never a path, cwd
  or content. Because there is nothing to hide, writes do not pass through the ledger's `Redact`
  seam, which is typed on ledger entries. #67 can add a seam if a policy needs one.
- **Concurrency:** one append-mode `write_all` per call, no locking. Two hooks racing on the same
  event may both write it; the reader collapses it. Distinct events are never lost.
- **Honest reading:** malformed lines, unknown schema versions and unknown kinds are counted
  separately, with line numbers, never dropped silently.

## Addendum: opting in, and refusing links (#94)

`.tally/` is part of the repository, and git tracks symbolic links. A checkout could ship
`.tally/captures.jsonl`, `.tally/ledger.jsonl` or `.tally` itself as a link to a file outside the
repository, and hook-driven writes would land there (security assessment F-01). Two rules close
that.

- **Writes need an opt-in.** `capture` and `record` write only when `.tally/` already exists as a
  real directory. Neither creates it. Without it, `capture` exits 0 silently and `record` exits 0
  with a one-line note that nothing was recorded. Git does not track an empty directory, so a
  repository opts in by committing a file inside it: `.tally/.gitkeep`, or an existing ledger.
  That gives every clone and worktree the directory.
- **Links are refused, never followed.** Every read and write of a default `.tally/*` path goes
  through one guarded accessor (`ledger::TallyDir`). That covers the ledger, the captures file and
  the `prices.toml` override. It refuses `.tally` if it is a link, is not a directory, or does
  not resolve inside the checkout it is opened for. It refuses any target that is not a regular
  file. Files are opened without following links, and the opened handle is checked again. A
  refusal is an error that names the path, never an empty read, so a planted link can't make a
  total look smaller without saying so.
- **No check-then-use gap on `.tally`.** On Unix the accessor holds a directory handle for
  `.tally` and opens every file relative to it (`openat`, through `rustix`, because std has no
  safe `openat` and the workspace forbids `unsafe`). Swapping `.tally` for a link after the check
  therefore changes nothing. On Windows the handle is held without delete sharing, so `.tally`
  cannot be renamed or replaced while files in it are opened.
- **The checkout root is the trust anchor.** The accessor defends `.tally` and the files in it,
  not the directories above the checkout.
- **An explicit `--ledger <path>` is exempt.** A path the user typed is the user's choice, links
  included. The default ledger's location is unchanged: the git top level's `.tally/`, so each
  worktree keeps its own ledger, as above. Captures and `prices.toml` stay in the main checkout's.
