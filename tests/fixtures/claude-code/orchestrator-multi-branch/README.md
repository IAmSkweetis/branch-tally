# orchestrator-multi-branch

Hand-written and synthetic. Relates to #2 and #7 (multi-branch sessions, non-`feat` prefixes).
Shape modelled on Claude Code 2.1.x, 2026-11-08.

One session, `s1`, seven calls in time order, input tokens 1 to 7 in that order: `main`, `feat/a`,
`feat/a`, `feat/b`, `refactor/c`, `feat/b`, `test/d`. The session returns to `feat/b` after
visiting `refactor/c`, and two of the branches use prefixes the original script's work-branch
pattern does not know.

A correct read reports `files_read` 1 and `records_emitted` 7, every skip counter 0. Per raw
branch: `main` 1 call / 1 input, `feat/a` 2 / 5, `feat/b` 2 / 10, `refactor/c` 1 / 5, `test/d`
1 / 7. A session claim cannot pick one branch here: a correct report must not hand all seven calls
to any single branch, and must label any branch total that relied on a timeline guess.
