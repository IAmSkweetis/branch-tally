# unc-share

Hand-written and synthetic. Relates to #38. A repository on a UNC share is given to git as the
verbatim form `\\?\UNC\srv\share\repo`. Claude Code names its projects directory from the cwd
`\\srv\share\repo`, giving `--srv-share-repo` (shape expected from the slug rule; not yet observed
on a real UNC share, 2026-11-10).

The only directory is `--srv-share-repo`, with one record `req_1`. Repo `\\?\UNC\srv\share\repo`
must discover it as the Repository root. Reading reports `files_read` 1, `records_emitted` 1.
