# replayed-history

Hand-written and synthetic. Relates to #3 (dedupe across roots) and the "replayed history" fact in
`docs/references/transcript_format.md`. Shape modelled on Claude Code 2.1.x (observed 2026-11-08).

A resumed session replays earlier requests. Layout (the directory is the projects root, repo
`C:\repo`):

- `C--repo/s1.jsonl`: `req_1`, `req_2`.
- `C--repo/s2.jsonl`: replays `req_1`, `req_2`, then `req_3`.
- `C--repo--claude-worktrees-wt1/s3.jsonl`: replays `req_3` (a duplicate across two roots).

A correct read of both discovered roots reports `files_read` 3, `records_emitted` 3 (`req_1`,
`req_2`, `req_3`), `duplicates_removed` 3, and every other counter 0. The first occurrence wins, so
`req_3` keeps `gitBranch` `main`.

Content fields hold the placeholder `PLACEHOLDER` only.
