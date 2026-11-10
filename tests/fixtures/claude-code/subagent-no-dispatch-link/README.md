# subagent-no-dispatch-link

Hand-written and synthetic. Relates to #6 and Governance II. Observed in Claude Code 2.1.x on
2026-11-09.

An orchestrator on `feat/x` (`s1.jsonl`, one call, input 1) has a dispatch line for `n1` only. `n1`
has one call (input 2); `n2` has two (input 5, 6). Both subagent files sit under `s1/subagents/`
and are stamped `worktree-agent-<id>` with a worktree `cwd`. Nothing in `s1.jsonl` links `n2`.

A correct read reports `files_read` 3, `records_emitted` 4, every skip counter 0, and one dispatch
`(s1, n1, feat/x)`. A correct rollup puts `n1` and the orchestrator call on `feat/x` (3 input),
keeps `n2` on its raw `worktree-agent-n2` stamp, and counts its 2 calls (11 input) as
unattributed. It does not guess a branch for it and does not look up live worktrees.
