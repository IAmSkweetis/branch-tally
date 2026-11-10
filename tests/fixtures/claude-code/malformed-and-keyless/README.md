# malformed-and-keyless

Hand-written and synthetic. Relates to #3 (skip counters; Governance II). `C--repo/s1.jsonl` has,
in order:

1. a valid record `req_1`;
2. a truncated JSON line (malformed);
3. a blank line (ignored, not counted);
4. `42`, valid JSON but not a record (malformed);
5. a user line with no `message.usage` (not a usage record; not counted anywhere);
6. a usage record with neither `requestId` nor `uuid` (`no_dedupe_key`);
7. a usage record with only `uuid` `u7` (emitted, keyed by the uuid fallback);
8. a second `u7` (duplicate).

A correct read reports `files_read` 1, `records_emitted` 2, `malformed_lines` 2, `no_dedupe_key` 1,
`duplicates_removed` 1, `synthetic` 0. The emitted `u7` record has its `request_id` set to `u7`.

Dispatch-shaped lines (added 2026-11-09): user lines after line 5 whose `toolUseResult` is a
string, an object without `agentId`, an empty `agentId` or a numeric `agentId` yield no dispatch;
two identical lines for agent `m1` yield one `(s1, m1, main)`. None changes the read counters.
