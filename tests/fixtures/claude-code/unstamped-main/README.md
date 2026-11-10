# unstamped-main

Hand-written and synthetic. From #37. Shape modelled on Claude Code 2.1.x, 2026-11-10.

Two sessions in the main checkout, each with a call that carries no `gitBranch` stamp:

- `s1`: `req_1` and `req_3` stamped `feat/x`, `req_2` unstamped between them.
- `s2`: `req_4` stamped `main`, `req_5` unstamped.

Input tokens are 1 to 5 in request order, so a record on the wrong side changes a total.

The reference looks up the main checkout in `git worktree list` for an unstamped record, so it
assigns `req_2` and `req_5` to whatever branch the main checkout has checked out when the report
runs. The golden generator's checkout is on `main`, so its claim report for `main` has 3 calls
(`req_2`, `req_4`, `req_5`). That is a snapshot of the present, not of when the calls were made.

A correct read reports `files_read` 2 and `records_emitted` 5, every skip counter 0. A correct
**report** run in strict mode keeps `feat/x` (2 calls), `main` (1 call) and `(none)` (2 calls)
apart. In claim mode the unstamped calls stay `(none)` and are reported as unattributed (#37):
`feat/x` still claims all of `s1` (3 calls, 6 input), because `feat/x` is that session's only work
branch, and `main` has only `req_4` (1 call, 4 input). That claim-mode `main` result is a listed
divergence from the reference.
