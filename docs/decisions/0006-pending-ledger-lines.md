# 0006. Record default-branch usage as pending and resolve it by appending

- **Status:** accepted
- **Date:** 2026-11-16
- **Issue:** #131. Amends [0002](0002-ledger-format.md) and [0005](0005-ledger-redaction.md).

## Context

The orchestrating session usually starts on `main`, and `/feature` cuts the branch partway through.
`record` writes a final branch at write time, so calls made before the cut land in
`.tally/ledger.jsonl` as `branch: main`, `basis: stamped`, `kept_by: exact`. `/feature` carries
those uncommitted lines onto the feature branch, and `ledger check` rejects every one as
`recorded for the base branch`. On #130 that was 18 lines, deleted by hand.

`ledger check` fails a PR whose head drops or changes any line on base, so a line can't be
rewritten once written. Report claim and timeline modes already attribute pre-branch `main`
records forward to the branch cut next (#7). The ledger had no equivalent.

## Decision

- **Pending line.** When `record` runs on the repository's default branch, each line it writes
  carries a new optional field, `"pending": true`, omitted when false. `recorded_for`, `branch`,
  `basis` and `kept_by` are left as attribution produced them, so the line still says exactly what
  `record` saw. Its key stays `(request_id, "main")`.
- **Resolution line.** A pending line is resolved by **appending** a new v1 `LedgerEntry` to
  `ledger.jsonl`, never by editing the pending one. The resolution copies the stored pending line
  (its already-hashed ids, `usage`, `model`, `ts`, `origin`, `subagent`, `redaction`, `mode`). It
  sets `recorded_for` and `branch` to the target branch and `kept_by: exact`, and adds
  `"resolved": {"from": "<default branch>"}`. Its `basis` names the evidence with an existing
  variant: `timeline-forward` (the session's timeline), `dispatcher-rolled-up` (a subagent), or
  `captured` (a capture event). It never needs the salt to hash anything, because it copies hashed
  values.
- **Already covered.** A pending `(rid, main)` counts as resolved, with nothing appended, when a
  non-pending line for the same `rid` already exists on another branch (for example a claim-mode
  `record` on the feature branch). That avoids a second, conflicting `(rid, target)` line.
- **No schema bump.** `v` stays 1. `LedgerEntry` does not deny unknown fields, so older readers
  ignore `pending` and `resolved` and read a resolution as an ordinary line for its branch. Every
  existing committed line has neither field and parses and checks exactly as before.
- **Resolution evidence.** The timeline over the live records (`attribute_timeline` with the
  default work-branch pattern), with live ids hashed under the salt whose fingerprint matches the
  pending line; and the captures: a session's first capture with a work branch at or after the
  record's `ts`. Only `TimelineForward` (the branch cut next in that session), `DispatcherRolledUp`
  and that capture rule resolve.
- **What stays pending.** No `ts`; a timeline result of `TimelineAmbiguous`, `Stamped` (no work
  branch in the session), `SubagentUnlinked` or `TimelineCarried` (the session returned to `main`
  after a work branch: it is never assigned back to that earlier branch); transcripts and captures
  disagreeing; no salt matching the line's fingerprint; the request already resolved to a different
  branch. Each is counted by reason and reported as uncertain (Governance II). None is guessed.
- **Where resolution runs.** Automatically at the end of `record` when the branch is not the
  default, and explicitly with `branch-tally ledger resolve [--branch B] [--dry-run]`. Both resolve
  **only** pending lines whose target is the current or named branch, so a PR carries resolutions
  for itself only. There is no `--all`.
- **`ledger check`.** It judges only lines new on head. A new line for the base branch with
  `pending` set is a **note, not a failure**: the report counts it and prints
  `note: N pending line(s), usage uncertain`. A new base-branch line without `pending` still fails
  `RecordedForBaseBranch`. New rules: `InvalidLineKind` (both `pending` and `resolved`),
  `OrphanResolution` (no matching `(rid, from)` pending line on base or head),
  `ResolutionMismatch` (the resolution differs from its pending line in any field other than
  `recorded_for`, `branch`, `basis`, `kept_by`, `recorded_at`, `pending` and `resolved`, so
  `origin`, `subagent`, `agent_id`, `skill`, `agent`, `harness` and `mode` must match too),
  `ConflictingResolution` (the same `rid` resolved to a different branch on base or head), and
  `PendingNotOnBase` (a new line with `pending` set whose `recorded_for` is not the base branch).
- **Reporting.** `Summary` gains `pending` (lines, calls, tokens), `#[serde(default)]`: unresolved,
  uncovered pending lines across the ledger, deduplicated by request id, kept outside the totals.
  A pending line is in **no** branch's totals, the default branch's included: a report for the
  default branch, from the ledger or live, and the `project` report's per-branch rows all leave
  pending lines out, and they appear only in `pending`. A request the report already counts in its
  totals (for example a pre-cut call that timeline or claim mode attributes forward before
  `record` has resolved it) is left out of `pending`, so no call is counted twice. A report that
  counts no records but has unresolved pending lines (for example the default branch, when all its
  lines are pending) is still built, with zero totals and the pending usage, so the uncertain usage
  is never hidden behind a "no records" error. The `project` report shows the same pending usage
  beside its totals, never in them.
  Markdown prints it as a footnote beside the unstamped one, and JSON as a `"pending"` object
  beside `"unstamped"`.

## Consequences

- A PR from the standard `/feature` flow carries its pre-branch lines as pending plus their
  resolutions, and passes `tally-check` without hand-deletion.
- `tally-check` builds the checker from the base branch and the hooks run the installed release,
  so the PR that introduces this still needs its own pre-branch lines removed by hand once. The
  behaviour is shown on a real PR after a release, on the next `/feature` PR.
- `ledger check` sees only the ledger, never the transcripts. It proves a resolution is a faithful
  copy of a pending line and that no request is resolved twice, not that the evidence was real: a
  PR that adds both a pending line and its own resolution passes. A passing check means the ledger
  is consistent, not that its attribution is true.
- A repository whose `tally-check` pins an older release fails on pending lines, because they are
  still recorded for `main`. `recorded_for` stays truthful rather than holding a sentinel. The
  release notes call this out as a breaking change: upgrade `tally-check` with the binary.
