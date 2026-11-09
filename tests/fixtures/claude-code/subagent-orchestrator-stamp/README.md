# subagent-orchestrator-stamp

Hand-written and synthetic. Relates to #2 and #6. Evidence pattern from #6, observed in Claude Code
2.1.x on 2026-11-08.

`C--repo/s1.jsonl` holds two orchestrator calls on `feat/x`. The subagent file
`s1/subagents/agent-b1.jsonl` holds two calls (`req_b1`, `req_b2`) with `agentId` `b1`, the
parent's `sessionId`, `isSidechain` true, and a `cwd` of `C:\repo\.claude\worktrees\agent-b1` --
yet stamped `feat/x`, the orchestrator's branch, not the worktree's. The manifest keeps the
worktree alive on `chore/b1-data`: that is the trap, because nothing in these records names it.

A correct read reports `files_read` 2 and `records_emitted` 4, every skip counter 0, with the
stamps and the worktree `cwd` reported untouched. A correct report puts all four calls (10 input)
on `feat/x` and none on `chore/b1-data`, and says 7 of those input tokens (3 + 4) came from a
worktree subagent.
