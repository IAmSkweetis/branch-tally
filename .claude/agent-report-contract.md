# Agent Report Contract

The single source of truth for how every specialized agent reports back.

## Delivery rule

End your final message with **exactly one** fenced ```json block, and nothing after it. Prose above
the block is fine, and encouraged for human readers. The orchestrator parses only the last fenced
block. Never commit a report file: a committed file can't contain the SHA of the commit that
contains it.

## Shape

```json
{
  "agent": "Core Implementer",
  "status": "complete",
  "summary": "Attribution rolls each worktree subagent file up to its dispatcher's branch",
  "branch": "fix/issue-6-worktree-subagent-rollup",
  "worktree_path": ".claude/worktrees/...",
  "commits": ["abc1234", "def5678"],
  "files": { "created": [], "modified": [], "deleted": [] },
  "verification": [
    { "method": "cargo_test", "expectation": "red",   "command": "just test attribution", "exit_code": 100, "tests_run": 41, "evidence": "subagent_rolls_up_to_dispatcher_branch: left 0 right 3" },
    { "method": "cargo_test", "expectation": "green", "command": "just test",             "exit_code": 0,   "tests_run": 128, "evidence": null }
  ],
  "escalations": [],
  "user_facing_changes": [
    { "kind": "flag", "name": "--work-branch", "detail": "new; regex naming the branches that count as work branches (timeline mode only)" }
  ],
  "notes": null
}
```

## Field rules

- `agent` MUST be the agent's frontmatter `name:` **verbatim**, so a report can be traced to the
  definition that produced it.
- `commits` is an array, in order.
- `branch` and `worktree_path` let the orchestrator merge and clean up without inferring either.
- `verification` is an **array**, not an object, so a test-first agent can report its red run and
  its green run as two entries. That makes "did test-first actually happen" mechanically checkable.
- `tests_run` is REQUIRED on every `cargo_test` entry. `exit_code: 0` with `tests_run: 0` is a false
  green (see `docs/references/testing.md`), and requiring both numbers turns that trap into a
  detectable lie. For a green entry, `command` must be the full suite (`just test`), not a filter.
- `user_facing_changes` is **optional**: omit it, or leave it `[]`, when nothing a user can see
  changed. An agent that implements something users can see lists it here, so the Docs Author has an
  input to work from instead of reading the diff. Each entry has `kind` (`flag`, `config_key`,
  `install_option`, `output` or `other`), `name` (the flag, key, option or output field as the user
  types or reads it) and `detail` (one line: what it does, its default, and whether it is new,
  changed or removed). Never include transcript content. A report without the field stays valid.
- Evidence never contains transcript content, home-directory paths or secrets (Governance III). Use
  counts, ids and file names.
- Commits MUST follow Conventional Commits, and `branch` MUST follow Conventional Branch
  (`<type>/<description>`, lowercase and hyphenated). Name the branch for the change, not the
  agent. The agent is already recorded in `agent`.

## Taxonomy

These are closed sets. An agent that needs a value not listed here must escalate, not invent one.

**These sets are deliberately duplicated.** Every agent definition in `.claude/agents/` restates
`status`, the `verification` entry shape and the `files` object in its own Report section, and
every implementing agent restates `user_facing_changes`. In the
project this pattern comes from, linking to this page alone didn't hold: agents invented `status`
values (`completed`, `success`) and returned `verification` as an object. The price is that an
edit here is only half an edit. **Edit this page and every agent definition in the same
commit.** Otherwise the copies become what agents actually follow, while this page quietly
disagrees.

### `status`

- `complete`: the task is finished as scoped.
- `complete_no_changes`: swept, clean, nothing to change. This must be distinguishable from a
  no-op. A read-only auditor finding nothing wrong is a real success.
- `blocked`: a precondition was missing, or a rule forbade going on.
- `failed`: attempted, and did not succeed.

**Invariant: `blocked` and `failed` MUST each carry at least one escalation.**

### `verification[].method`

`cargo_test` | `clippy` | `fmt` | `dogfood` | `static_audit` | `none`

- `cargo_test` requires `expectation` (`red` or `green`), `command`, `exit_code` and `tests_run`.
- `dogfood` means running the built binary against real local transcripts, read-only. It requires
  `command` and `evidence`: the numbers it printed, never content.
- `none` requires `notes` saying exactly what was run and observed instead.

### `escalations[].kind`

- `missing_red_test`: implementation was requested with no failing test to implement against.
- `out_of_scope_defect`: a bug found outside the task's scope.
- `scope_boundary`: the task as given would require writing outside the agent's declared scope.
- `boundary_violation`: a pipeline stage reached past its neighbour (a source deciding branches, a
  renderer computing totals, core calling `gh`), or something called git directly instead of
  through the trait.
- `model_change_required`: a field or type in `model/` must change. `suggested_agent` names the
  Core Implementer holding the `model` slice.
- `needs_core_api`: the CLI or an integration needs something `branch-tally-core` doesn't expose.
  Raised instead of reimplementing it outside core.
- `fixture_required`: a test needs a transcript situation no fixture reproduces.
  `suggested_agent` names the Harness Source Author.
- `dependency_request`: a new crate, feature flag, version bump or toolchain change.
  `suggested_agent` names the Workspace Config Steward. `detail` says why the standard library or
  an existing dependency won't do.
- `file_outside_scope`: a file outside the agent's write scope must change. Name its owner in
  `suggested_agent`.
- `privacy_risk`: transcript content, a home path or a secret is about to be committed, logged or
  printed. Stop work and raise this.
- `overstated_claim`: output would state a number with more certainty than the data supports
  (Governance II).

### `escalations[].severity`

`blocker` | `violation` | `risk` | `info`

Each escalation also carries `detail` (required), and optionally `path` and `suggested_agent`.
