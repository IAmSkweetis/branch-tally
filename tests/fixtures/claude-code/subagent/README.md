# subagent

Hand-written and synthetic. Relates to #3 (source) and #6 (subagent attribution). Layout observed
in Claude Code 2.1.x on 2026-11-08.

- `C--repo/s1.jsonl`: `req_1`, branch `feat/x`.
- `C--repo/s1/subagents/agent-a1.jsonl`: `req_a1`, `req_a2`, with `agentId` `a1`,
  `isSidechain` true, `sessionId` `s1` (the parent's) and `gitBranch` `worktree-agent-a1`.

A correct read reports `files_read` 2 and `records_emitted` 3, every skip counter 0. The two
subagent records carry `parent_session` `s1`, `agent_id` `a1`, `is_sidechain` true, and the raw
`worktree-agent-a1` branch, untouched. The main record has no `parent_session` and no `agent_id`.

Dispatch lines (added 2026-11-09, Claude Code 2.1.x): the parent session file also holds one
`type: "user"` line per subagent with `toolUseResult.agentId` set, the parent's branch stamp
`feat/x` and the parent `sessionId`. They carry no usage, so read counts are unchanged. A correct
dispatch pass reports one `(s1, <agent>, feat/x)` link per subagent, and the rollup puts every
subagent call on `feat/x`.
