---
name: rust-tests
description: Run and write branch-tally's Rust tests test-first. Covers the commands, how to show a genuine red before a green, the false-green traps, snapshot review and fixture rules. Use before writing or running any test, and whenever a result looks too good.
metadata:
  author: branch-tally
---

# Rust tests

The policy is in `docs/references/testing.md`. This is the checklist.

## Commands

```bash
just test                 # cargo nextest run --workspace, then cargo test --workspace --doc
just test <filter>        # nextest filter: a module, a test name, a binary
just ci                   # every gate CI runs
```

Before the justfile exists: `cargo nextest run --workspace [filter]` and
`cargo test --workspace --doc`.

Don't use plain `cargo test <filter>` to prove green. A filter that matches nothing exits 0.

## Red, then green

1. Write the test in the right place:
   - unit tests go in `#[cfg(test)] mod tests` in the same file;
   - integration tests go in `crates/<crate>/tests/<module>.rs`;
   - end-to-end tests go in `crates/branch-tally/tests/`.
2. Stub the code under test so it **compiles**, with `todo!()` or a deliberately wrong value.
3. `just test <filter>`. **Read the failure.** It should be your assertion, or the `todo!()` you
   placed, failing for the reason you expect. A panic in setup or a missing fixture isn't red yet.
4. Record the red run: the command, the exit code (nextest exits 100 on a test failure), the test
   count, and the failing line.
5. Implement the smallest change. `just test <filter>` until green.
6. `just test` across the whole workspace. Record the exit code **and the test count**.

Never weaken an assertion, add `#[ignore]`, or accept a snapshot unread to get to green.

## Before you trust a green

- Did the count go **up** by the number of tests you added? If not, your test file may not be
  compiled. Check it is under `tests/`, or declared with `mod`.
- Did doctests run? `just test` runs them separately, because nextest skips them.
- Is the test behind a cargo feature? Run with that feature enabled.
- Does the fixture still exercise the path? Assert a count that proves it: "3 subagent files read",
  not just "result is Ok".

## Snapshots

- A changed snapshot is a failing test until a human reads the diff.
- Review new or changed snapshots one at a time. Never accept them all wholesale.
- CI never updates snapshots. A pending `.snap.new` file fails the run.

## Fixtures

- They live in `tests/fixtures/<harness>/<scenario>/`, each with a `README.md`.
- They are **scrubbed or synthetic only**. Never paste a real transcript line.
- Only the Harness Source Author writes them. Anyone else who needs a new scenario raises
  `fixture_required`.

## Reporting

Your `verification` array needs a red `cargo_test` entry and a green one. Both carry `command`,
`exit_code` and `tests_run`. The green one is the full `just test`, not a filter.
