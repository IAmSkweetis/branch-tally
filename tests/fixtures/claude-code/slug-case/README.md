# slug-case

Hand-written and synthetic. Relates to #3. On Windows the same repo can appear as `D--x` or `d--x`
(observed 2026-11-08), so slugs compare case-insensitively.

The only directory is `c--repo`, with one record `req_1`. Repo `C:\repo`, `c:/repo`, `/c/repo`
and `/C/repo` must all discover it as the Repository root. Reading reports `files_read` 1,
`records_emitted` 1, every skip counter 0.
