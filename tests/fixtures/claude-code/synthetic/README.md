# synthetic

Hand-written and synthetic. Relates to #3. Claude Code writes local interrupt and error records
with model `<synthetic>` and zero usage (observed 2026-11-08, 2.1.x).

`C--repo/s1.jsonl` holds `req_1` (a real call) and `req_2` (model `<synthetic>`).

A correct read reports `files_read` 1, `records_emitted` 1, `synthetic` 1, every other counter 0.
