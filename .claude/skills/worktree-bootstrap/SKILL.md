---
name: worktree-bootstrap
description: Prepare a fresh git worktree of branch-tally so its test results can be trusted. Use immediately after creating or entering a worktree, before running the suite or making changes, and whenever a suite in a new worktree reports zero tests or an implausibly low count.
metadata:
  author: branch-tally
---

# Worktree bootstrap

A fresh worktree checks out the source but not `target/`, which is gitignored and per-worktree. The
first build compiles everything from scratch. This skill starts that build early, confirms the
toolchain and hooks came through, and records a baseline test count to compare against.

## Steps

```bash
git config core.hooksPath            # expect: .githooks
rustup show active-toolchain         # expect the version pinned in rust-toolchain.toml
cargo nextest --version              # the runner. Install once per machine if missing.
just test                            # first build plus the full suite
```

Record the exit code **and the test count**. Compare the count with the number the orchestrator
reported for the feature branch's baseline. If it's zero or far lower, the run didn't pass, it
stopped. See the false-green table in `docs/references/testing.md` before investigating anything
else.

Until the workspace exists (#3), there is nothing to build. Confirm the hooks path and the
toolchain, and say so in your report.

## What you do NOT need to do

- **Fetch dependencies by hand.** `cargo` fetches them on the first build, from the shared
  `~/.cargo` registry cache.
- **Share a target directory with other worktrees.** Don't set `CARGO_TARGET_DIR` to a shared path.
  Parallel agents would serialize on cargo's lock, and one agent's `cargo clean` would wipe the
  others' builds. A slow first build is the accepted cost.

## Hooks

`core.hooksPath` is the relative path `.githooks`, which git resolves against the root of the
working tree running the hook. **A worktree runs its own branch's hooks.** If your branch changes
`.githooks/`, your commits exercise the new hooks immediately. That's useful for testing them, and
worth knowing if a hook you're editing is currently broken.

If `git config core.hooksPath` prints nothing, the hooks aren't enabled in this clone. Tell the
orchestrator. Don't enable them silently on the user's behalf.

## Working in the worktree

- Use a filtered run (`just test <module>`) for the inner loop, and the full `just test` before you
  hand work back.
- Prefer a WIP commit over `git stash`. The stash is shared across every worktree.
- Commit before finishing, but do **not** push. The commit is the durability boundary.
- Conventional Commits **and** Conventional Branch, per `docs/references/dev_workflow.md`.
- `~/.claude/projects/` is read-only to you. Never write there.

## Files that collide across parallel worktrees

| File | Why it hurts | Owner |
| ---- | ------------ | ----- |
| `Cargo.toml`, `Cargo.lock` | A clean merge can still change what gets built | Workspace Config Steward |
| `crates/branch-tally-core/src/lib.rs` | Every new module is declared here | Core Implementer holding `model` |
| `crates/branch-tally-core/src/model/` | Shared types. A new field breaks other branches' constructors | Core Implementer holding `model` |

If your task needs one of these, escalate (`dependency_request` or `model_change_required`) instead
of editing it alongside your own work.
