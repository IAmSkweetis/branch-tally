# 0001. Implement branch-tally in Rust

- **Status:** accepted
- **Date:** 2026-11-08
- **Issue:** #1

## Context

branch-tally grew out of `token_report.py`, a single-file Python script run with `uv`. It reports a
pull request's agent-token cost from the local Claude Code transcripts. The tool now has to ship
in several forms: a local CLI, a Claude Code plugin and skill, a GitHub Action and, later, a
service container. Each of those runs the same core on someone else's machine or CI runner.

It also has to read a lot of data. A busy repository's transcript tree runs to gigabytes of JSONL,
and the report runs on every PR.

## Decision

branch-tally is implemented in **Rust**, stable channel, edition 2024.

- **Toolchain:** the first pin is Rust **1.99.1**, the latest stable release when this was
  decided. It goes in `rust-toolchain.toml` when the workspace lands (#3).
- **MSRV:** the root `Cargo.toml` declares `rust-version = "1.99.1"` to match. The `msrv` CI job
  fails until it is declared.
- **Moving the pin:** a toolchain bump is a version bump. It goes through the Workspace Config
  Steward like any other ([agents.md](../references/agents.md)), and needs no new decision record.
- **The Python script stays.** `reference/token_report.py` is frozen as the reference
  implementation. Its `--json` output is the golden output the Rust port is tested against (#2,
  #4). It is retired once Rust is the source of truth
  ([testing.md](../references/testing.md#parity-with-the-python-reference)).

## Why Rust

- **One static binary per OS.** The skill, the Action, a container and local installs all run it
  with no runtime to install first.
- **One release pipeline.** cargo-dist builds shell and PowerShell installers, a Homebrew formula,
  an npm wrapper and GitHub Release archives from the same workflow (#10).
- **Fast over large transcript trees.** Streaming multi-GB JSONL is cheap, so the report can run
  on every PR without becoming the slow step.
- **Useful prior art.** The comparable multi-harness usage trackers (tokscale, toktrack, splitrail,
  toll) are written in Rust. Their transcript parsers are a useful reference when adding
  harnesses (#14).

## Alternatives considered

- **Python (the status quo).** Fastest to iterate on, and the code already exists. But every
  runner and every harness would need `uv` or a Python install, and distribution would mean either
  asking users for a runtime or bundling one.
- **TypeScript.** `npx` distribution like ccusage, and a native JavaScript Action. But it needs
  Node everywhere the tool runs, and gives up the single static binary.

## Consequences

- Workspace layout is a library crate, `branch-tally-core`, and a thin binary crate,
  `branch-tally` ([project_layout.md](../references/project_layout.md)). It lands in #3.
- Every dependency is a supply-chain and binary-size decision, reviewed by the Workspace Config
  Steward and checked by `cargo deny`.
- Until the Rust `report` command can produce this repository's own PR token report (#4),
  `just token-report` keeps running the Python script.
- Contributors need a Rust toolchain. `rustup` reads `rust-toolchain.toml` and installs the pinned
  version automatically.
