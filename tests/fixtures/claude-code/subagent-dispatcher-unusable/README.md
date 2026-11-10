# subagent-dispatcher-unusable

Hand-written and synthetic. Relates to #6 and Governance II. Observed in Claude Code 2.1.x on
2026-11-09.

- `s1.jsonl`: orchestrator on `feat/x` (1 call, input 1), dispatches `g1` (stamp `feat/x`).
- `s1/subagents/agent-g1.jsonl`: `g1` (1 call, input 2, stamp `worktree-agent-g1`) which itself
  dispatches `g2`. That dispatch line sits inside the subagent file, carries `sessionId` `s1`
  and the stamp `worktree-agent-g1`.
- `s1/subagents/agent-g2.jsonl`: `g2` (1 call, input 3, stamp `worktree-agent-g2`).
- `s2.jsonl`: a second session on `feat/x` (1 call, input 4) whose dispatch line for `g3` has no
  `gitBranch` at all.
- `s2/subagents/agent-g3.jsonl`: `g3` (1 call, input 5, stamp `worktree-agent-g3`).

A correct read reports `files_read` 5, `records_emitted` 5, every skip counter 0, and three
dispatches: `(s1, g1, feat/x)`, `(s1, g2, worktree-agent-g1)` and `(s2, g3, none)`, with the
stamps reported raw. A correct rollup puts `g1` on `feat/x` (3 input with the orchestrator call).
`g2` (dispatcher stamp is itself a `worktree-agent-` stamp) and `g3` (no dispatcher stamp) keep
their raw stamps and are counted as unattributed: 2 calls, 8 input. Nested dispatch is not
chased up the chain.
