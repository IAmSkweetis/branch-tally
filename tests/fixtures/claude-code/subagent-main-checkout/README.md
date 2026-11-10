# subagent-main-checkout

Hand-written and synthetic. Relates to #6. Observed in Claude Code 2.1.x on 2026-11-09.

An orchestrator on `feat/x` (`s1.jsonl`, one call, input 1) dispatches `i1`, which works in the
main checkout (`cwd` `C:\repo`, no worktree) and stamps `feat/x`. It has two calls (input 2, 3).

A correct read reports `files_read` 2, `records_emitted` 3, every skip counter 0, and one dispatch
`(s1, i1, feat/x)`. A correct rollup puts all three calls (6 input) on `feat/x`, with the
subagent in workspace `main` (not a worktree), subagent `i1`.
