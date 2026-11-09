# subagent-switches-branch

Hand-written and synthetic. Relates to #2 and #6. Observed in Claude Code 2.1.x on 2026-11-08.

An orchestrator on `feat/x` (`s1.jsonl`, one call, input 1) dispatches two subagents that each
start stamped `worktree-agent-<id>` and then switch to their own branch before finishing. `d1`:
two calls stamped `worktree-agent-d1` (input 2, 3) then two on `chore/d1-data` (4, 5). `d2`: one
call stamped `worktree-agent-d2` (6) then two on `chore/d2-data` (7, 8). All have the parent's
`sessionId` and a worktree `cwd`. The worktrees are removed, so the manifest lists none.

A correct read reports `files_read` 3 and `records_emitted` 8, every skip counter 0, with each
stamp reported as written (a file's stamp changes mid-file). In strict mode `chore/d1-data` and
`chore/d2-data` show their own calls only. A correct claim-mode report for `feat/x` includes all
eight calls (36 input) as dispatcher work. A report for either `chore/` branch must not take
calls the orchestrator owns without saying so.
