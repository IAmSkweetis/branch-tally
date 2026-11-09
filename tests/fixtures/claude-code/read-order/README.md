# read-order

Hand-written and synthetic. Relates to #4 (parity with `token_report.py`). Shapes observed in
Claude Code 2.1.x on 2026-11-08; the read rules come from `load_records` and `transcript_roots`
in the reference.

- `C--repo/zzz.jsonl` and `C--repo/aaa/subagents/agent-x.jsonl` both carry `req_dup`, on branches
  `from-top` and `from-sub`.
- `C--repo/bbb/deep/x.jsonl`: `req_deep`, a `.jsonl` nested in a directory that is not
  `subagents/`.
- `C--repo--claude-worktrees/s2.jsonl` (`req_bare`) and `C--repo--claude-worktreesX/s3.jsonl`
  (`req_x`): worktree roots whose names have no `-` after `--claude-worktrees`, or no suffix at all.

A correct read walks each root in one pass sorted by path parts, so `aaa/...` is read before
`zzz.jsonl`: `req_dup` is kept from the subagent file (`from-sub`, parent session `s1`) and the
top-level copy is a duplicate. `req_deep` is read, with no parent session and its own `sessionId`.
Both extra roots are discovered as worktrees. Total: 5 records, 5 files, 1 duplicate removed.
