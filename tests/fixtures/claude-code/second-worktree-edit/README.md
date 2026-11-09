# second-worktree-edit

Hand-written and synthetic. Relates to #2 and #7. Observed in Claude Code 2.1.x on 2026-11-08.

One session, `s1`, in the main checkout, started on `feat/a`. Calls 1 and 2 have `cwd` `C:\repo`
and are stamped `feat/a`. The session then edits branch B through a second worktree, `feat-b`
(`C:\repo\.claude\worktrees\feat-b`): calls 3 and 4 have that `cwd` but are still stamped
`feat/a` (the stamp lags), and call 5 is stamped `feat/b`. Input tokens are 1 to 5 in order.

A correct read reports `files_read` 1 and `records_emitted` 5, every skip counter 0, with `cwd`
and stamp reported untouched. Strict per raw stamp: `feat/a` 4 calls / 10 input, `feat/b` 1 call /
5. A correct report must not silently move calls 3 and 4 to `feat/b` on a `cwd` guess; if it
does, it says so and the heuristic is tested.
