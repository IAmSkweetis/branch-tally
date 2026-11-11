# 0003. Publish a user manual with mdBook on GitHub Pages

- **Status:** accepted
- **Date:** 2026-11-10
- **Issue:** none

## Context

branch-tally's user-facing documentation lived in the README: install commands, installer options
and the config file. As the CLI, the ledger and the installers grew, the README could no longer
hold a full reference, and nothing kept it in step with the code. The project needs a manual that
users can browse, published from this repository.

The candidate tools were mdBook, mkdocs-material and Zensical. mkdocs-material is in maintenance
mode, and its successor Zensical is young. Both are Python, which this repository allows only as
`uv`-run tooling in `scripts/`.

## Decision

- **mdBook** builds the manual. It is the Rust ecosystem's documentation tool (it builds The Rust
  Book), ships as one static binary, and reads plain Markdown plus a `SUMMARY.md`. The pinned
  binary is installed in CI the same way as the repository's other tools.
- **Sources live in `docs/manual/`**: `book.toml` at its root, and the pages under
  `docs/manual/src/`. `docs/references/` stays the contributor and agent reference, and
  `docs/manual/` is for users.
- **Diagrams are Mermaid** fenced blocks in the Markdown, rendered on the site by the
  `mdbook-mermaid` preprocessor. They diff as text, and GitHub renders them when browsing the
  repository.
- **GitHub Pages hosts it**, deployed by a workflow on push to `main`. The PR CI builds the book,
  so a broken manual can't land.
- **The manual documents what ships on `main`.** Planned features are marked pending, not
  described. A PR that changes a flag, a config key or an install method updates the manual in
  the same PR.
- **The README links to the manual.** It keeps the one-line install commands and drops the
  details the manual now owns.
- **The manual does not discuss the original Python script.** Users never had it and have no
  context for it.

## Consequences

- The manual is one more thing to keep current. The upkeep rule in
  [dev_workflow.md](../references/dev_workflow.md) and the CI build hold it to the code.
- `book.toml`, the build recipe and the Pages workflow belong to the Repository Tooling Steward.
  A page under `docs/manual/src/` belongs to whichever agent changed what it describes, the same
  rule as `docs/references/`.
- Adding Mermaid means a second pinned binary, `mdbook-mermaid`, whose version has to stay
  compatible with mdBook's.
