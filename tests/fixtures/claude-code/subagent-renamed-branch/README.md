# subagent-renamed-branch

Hand-written and synthetic. Relates to #6. Shape observed in Claude Code 2.1.x on 2026-11-09.

An orchestrator on `feat/x` (`s1.jsonl`, one call, input 1) dispatches `e1` (a dispatch line,
branch `feat/x`). `e1` ran in `C:\repo\.claude\worktrees\agent-e1`: two calls stamped
`worktree-agent-e1` (input 2, 3), then it renamed its branch and made one call stamped
`chore/x-part` (input 4). The worktree still exists: the manifest lists it on `chore/x-part`.

A correct read reports `files_read` 2, `records_emitted` 4, every skip counter 0, and one dispatch
`(s1, e1, feat/x)`. A correct rollup puts all four calls (10 input) on `feat/x` under workspace
`agent-e1`, subagent `e1`. In strict mode `chore/x-part` shows only its own call (4 input).
