# main-then-branch

Hand-written and synthetic. Relates to #2 and #7 (branch timeline). Shape modelled on Claude Code
2.1.x, 2026-11-08.

One session, `s1`, in the main checkout. The first two calls (`req_1`, `req_2`) are stamped `main`;
the session then cuts a work branch and the last three (`req_3` to `req_5`) are stamped
`feat/new`. Input tokens are 1 to 5 in request order, so a record on the wrong side of the cut
changes a total.

A correct read reports `files_read` 1 and `records_emitted` 5, every skip counter 0. A correct
**report** run in strict mode keeps `main` (2 calls, 3 input) and `feat/new` (3 calls, 12 input)
apart. In claim mode for `feat/new` the two leading `main` calls are attributed forward to the
branch, and the report says the heuristic was used: 5 calls, 15 input.
