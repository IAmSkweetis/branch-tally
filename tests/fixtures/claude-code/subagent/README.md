# subagent

Hand-written and synthetic. Relates to #3 (source) and #6 (subagent attribution). Layout observed
in Claude Code 2.1.x on 2026-11-08.

- `C--repo/s1.jsonl`: `req_1`, branch `feat/x`.
- `C--repo/s1/subagents/agent-a1.jsonl`: `req_a1`, `req_a2`, with `agentId` `a1`,
  `isSidechain` true, `sessionId` `s1` (the parent's) and `gitBranch` `worktree-agent-a1`.

A correct read reports `files_read` 2 and `records_emitted` 3, every skip counter 0. The two
subagent records carry `parent_session` `s1`, `agent_id` `a1`, `is_sidechain` true, and the raw
`worktree-agent-a1` branch, untouched. The main record has no `parent_session` and no `agent_id`.
