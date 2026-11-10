# subagent-shared-worktree

Hand-written and synthetic. Relates to #6. Observed in Claude Code 2.1.x on 2026-11-09.

An orchestrator on `feat/x` (`s1.jsonl`, one call, input 1) dispatches `h1` and `h2`. Both ran in
the same worktree, `C:\repo\.claude\worktrees\wt-shared`, and both stamp `feat/x`. `h1` has two
calls (input 2, 3) with `attributionAgent` `explorer`; `h2` has one (input 4) and no stamped
agent type.

A correct read reports `files_read` 3, `records_emitted` 4, every skip counter 0, and two
dispatches, both `feat/x`. A correct rollup puts all four calls (10 input) on `feat/x` with two
breakdown rows in one workspace, `wt-shared`: subagent `explorer` (2 calls) and subagent `h2`
(1 call, falling back to the agent id).
