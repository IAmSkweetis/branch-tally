# worktree-and-siblings

Hand-written and synthetic. Relates to #3 (discovery). Slug rules observed 2026-11-08; see
`docs/references/transcript_format.md`. The directory is the projects root; the repo is
`C:\repo`, whose slug is `C--repo`.

| Directory | Discovered as |
| --------- | ------------- |
| `C--repo` | Repository |
| `C--repo--claude-worktrees-feat-a` | Worktree |
| `C--Repo--Claude-Worktrees-b` | Worktree (case differs) |
| `C--repo-cardgame` | excluded: a different repo sharing a prefix |
| `C--other` | excluded: unrelated |
| `notes.txt` | ignored: a file, not a directory |

A correct discovery returns exactly the first three. Reading them reports `files_read` 3 and
`records_emitted` 3, every skip counter 0.
