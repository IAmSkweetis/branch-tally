# 0005. Redact the committed ledger with a keyed hash under a per-repository salt

- **Status:** accepted
- **Date:** 2026-11-15
- **Issue:** #67 (Security Assessment F-09). Amends [0002](0002-ledger-format.md).

## Context

`branch-tally record` writes `.tally/ledger.jsonl` into the repository, and the repository may be
public. Every write passed through the `ledger::Redact` seam, but the only implementation was
`Identity`. A committed ledger kept, per API call, the request id, the session id, the agent id,
the agent and skill names, the model, branch names and a timestamp. In a public repository that is
a permanent per-call activity record (security assessment F-09).

Report redaction from #9 can't be reused. It renumbers ids per report (`s1`, `s2`, …). The ledger is
append-only, written by independent `record` runs on different branches and machines, and
union-merged, so per-run numbers would collide.

The assessment's first item, captures written to repositories that never opted in, was already
closed by #94: `capture` and `record` write only where `.tally/` exists (0002's opt-in addendum).

## Decision

- **Keyed hash.** With ledger redaction on, `record` replaces each identifier with
  HMAC-SHA256(salt, value), using `ring` (already a dependency). The value is written as
  lowercase hex, truncated to 32 characters (128 bits). The same input and salt always give the
  same output, so dedupe, the `(request_id, recorded_for)` key and the dispatcher roll-up, which
  links session ids to agent ids, keep working across runs, branches and machines.
- **What is hashed.** `request_id`, `session_id` and `agent_id`, plus every session or agent id
  inside `subagent` tags. Agent and skill names (`agent`, `skill`) are hashed only when a separate
  names opt-in is on. Nothing else changes. `ts` and `recorded_at` keep full precision, so the
  conflict rule ("latest `recorded_at` wins") is unchanged. Model, branches, basis, selection,
  origin and usage are unchanged too, so **totals are identical** to an unredacted ledger.
- **The salt.** 32 random bytes from `ring::rand::SystemRandom`, stored as hex in `.tally/salt` in
  the **main checkout** (like the captures file and `prices.toml`), so every worktree on a machine
  uses the same salt. The first redacted `record` creates it if it is missing, through the guarded
  `TallyDir` accessor: no links followed, a regular file only, created exclusively so two racing
  runs can't both write it. It is committed with the ledger, so every clone uses the same salt.
  A salt file that exists but is malformed is an error, never silently replaced.
- **Marking.** A redacted line carries `redaction`, a short fingerprint of the salt (the first 16
  hex characters of SHA-256 over the salt bytes). An unredacted line has no such field. The
  marker lets a reader tell the modes apart, and tell two salts apart.
- **Settings.** `record` has its own settings, separate from `report`'s `redact` /
  `redact-names`: config keys `ledger-redact` (default **true**) and `ledger-redact-names`
  (default false), with matching `record` flags. `report`'s defaults are unchanged.
  Redaction-by-default changes what `record` writes, so this is a breaking change.
- **Live union.** When `report --from-ledger` unions live transcripts into a redacted ledger, the
  live records pass through the same keyed hash before the union, so one call is never counted
  twice under two spellings.
- **Mixing is counted and noted.** A ledger holding unredacted lines and redacted lines, or lines
  redacted under more than one salt fingerprint, is read normally. The report says how many lines
  of each kind it saw, and that a call recorded once in each form may be counted twice
  (Governance II). It is never refused or hidden.
- **Backfill.** `branch-tally ledger redact` rewrites an existing ledger's unredacted lines with
  the current salt and settings, so they collapse into their redacted twins. It is the one
  deliberate exception to append-only. It is explicit, user-run, never automatic. It writes
  through a temporary file in the same directory and replaces the ledger only on success. It
  rewrites only lines that parse, and it counts and leaves untouched any it can't parse.

## Why

- **A keyed hash, not truncation.** It is deterministic and practically collision-free. Under a
  per-repository key, the same id in two repositories' ledgers can't be linked.
- **The salt is committed, and that's deliberate.** Every clone and machine has to produce the same
  hashes, or union merges double count. So the salt is not a secret: anyone holding a candidate id,
  such as a request id from their own transcript, can confirm whether it appears. The protection is
  against reading identifiers off a public repository and linking them across repositories, not
  against a holder of the original transcript.
- **Times are kept** because the conflict rule needs `recorded_at`, and the user chose not to
  coarsen `ts`.

## Consequences

- Two machines that each create a salt before either commits produce two `.tally/salt` files. Git
  reports a conflict on merge, which is loud. If the conflict is resolved by keeping one side, the
  other side's lines keep the other fingerprint, and the report counts them as mixed.
- A redacted ledger can't be checked against local transcripts without the salt. Anything that
  compares ledger keys with live request ids hashes the live ids first.
- `.tally/captures.jsonl` still holds raw session and agent ids, which attribution needs at
  `record` time. Captures are written only in opted-in repositories (#94). Hashing them is out of
  scope here.
