# subagent-removed-worktree

Hand-written and synthetic. Relates to #2 and #6 (the fixture #6 asks for). Observed in Claude
Code 2.1.x on 2026-11-08.

An orchestrator on `feat/x` (`s1.jsonl`, two calls, input 1 and 2) dispatched three subagents,
`c1` (2 calls), `c2` (1) and `c3` (2), each in its own worktree. Their files sit under
`s1/subagents/` with the parent's `sessionId`, and every record is stamped `worktree-agent-<id>`
with a worktree `cwd`. The worktrees were removed after the work merged, so the manifest lists
none: the stamp resolves to no branch.

A correct read reports `files_read` 4 and `records_emitted` 7, every skip counter 0. Subagent
input totals are `c1` 7, `c2` 5, `c3` 13 (25 in all), under the raw `worktree-agent-<id>` stamps. A
correct report attributes all seven calls to `feat/x` (28 input) via the dispatching session, not
by guessing, and says 25 of that came from worktree subagents. Dropping the five is the bug.

Dispatch lines (added 2026-11-09, Claude Code 2.1.x): the parent session file also holds one
`type: "user"` line per subagent with `toolUseResult.agentId` set, the parent's branch stamp
`feat/x` and the parent `sessionId`. They carry no usage, so read counts are unchanged. A correct
dispatch pass reports one `(s1, <agent>, feat/x)` link per subagent, and the rollup puts every
subagent call on `feat/x`.
