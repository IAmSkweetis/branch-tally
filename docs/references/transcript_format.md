# Claude Code transcript format

What branch-tally reads from Claude Code, and the facts about it that attribution depends on.
Everything here was **observed**, not documented by Anthropic. The format can change between Claude
Code versions, so each fact says when and where it was seen. If you find one has changed, update
this page and add a fixture for the new shape. Keep a fixture for the old shape while transcripts
in that shape can still exist (`cleanupPeriodDays`, default 30).

Observed on 2026-11-08, Claude Code 2.1.309 and 2.1.319, in the dodgeball-cardgame repository,
unless noted otherwise.

## Where transcripts live

- `~/.claude/projects/<slug>/`, where `<slug>` is the session's **cwd** with every character
  outside `[A-Za-z0-9]` replaced by `-`.
- **Slug case is not stable on Windows.** `d--workspaces-dodgeball` and
  `D--workspaces-dodgeball-game` both exist side by side. Compare slugs case-insensitively.
- A sibling repository can share a prefix (`dodgeball` vs `dodgeball-cardgame`). Match a slug
  exactly, or by an explicit worktree suffix, never by bare prefix.
- A worktree **entered as its own session** gets its own directory, whose slug is the repo slug
  plus `--claude-worktrees-<name>`. (Inherited from `token_report.py`. None existed on the machine
  this page was written on.)
- Transcripts are deleted after `cleanupPeriodDays` (default 30). Data not captured by then is
  gone, which is what the ledger (#8) and the pre-purge snapshot (#13) exist for.

## Files

| File | Holds |
| ---- | ----- |
| `<slug>/<session-id>.jsonl` | The main session's records, one JSON object per line |
| `<slug>/<session-id>/subagents/agent-<agentId>.jsonl` | One file per subagent dispatched by the Agent tool |

## Records that carry usage

Only records with `message.usage` matter. The fields branch-tally reads:

| Field | Meaning | Notes |
| ----- | ------- | ----- |
| `requestId` | The API request | **Dedupe key.** A resumed session replays its history, so the same request appears more than once. Without dedupe, totals inflate by about 2×. Fall back to `uuid` when absent. The dedupe set spans every transcript root. |
| `sessionId` | The session | For a subagent, this is the **parent's** session id. |
| `agentId` | The subagent | Present on subagent records. Matches the file name. |
| `isSidechain` | `true` for subagent records | |
| `timestamp` | ISO-8601, UTC | |
| `cwd` | The working directory of the call | For a worktree subagent this is the worktree path. **Both `D:\workspaces\…` and `/d/workspaces/…` forms appear for the same worktree**, so normalize before comparing. |
| `gitBranch` | The branch Claude Code stamped on the record | **Not reliable for worktree subagents.** See below. |
| `attributionSkill`, `attributionAgent` | Which skill or named agent made the call | |
| `message.model` | Model id | May carry suffixes such as `[1m]`. `<synthetic>` marks local interrupt and error records with zero usage, which should be skipped. |
| `message.usage.input_tokens`, `output_tokens`, `cache_read_input_tokens` | Token counts | |
| `message.usage.cache_creation.ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens` | Cache writes by TTL | Priced differently. Fall back to `cache_creation_input_tokens` as 5m when the breakdown is absent. |
| `message.usage.output_tokens_details.thinking_tokens` | Thinking share of output | Informational. Already included in `output_tokens`. |

Never read `message.content` or any other content field (Governance III).

## How the source reads these (`sources/claude_code/`, 2026-11-08)

Choices the parser makes where the format is silent. Fixtures for each are in
`tests/fixtures/claude-code/`.

- **Discovery.** A directory under the projects root is the repository's own when its name equals
  the repo slug, and a worktree's when it is the slug plus `--claude-worktrees-` and a non-empty
  suffix, both compared case-insensitively. Anything else is excluded, including a sibling repo
  that shares a prefix. A repo path shaped like `/d/x` yields two candidate slugs, the literal one
  and the `D:` drive one, so both cwd forms find the same directory. A leading `\\?\` is stripped.
- **Reading.** `<root>/*.jsonl` are main sessions. `<root>/<session>/subagents/agent-*.jsonl` are
  subagent files, and the directory name becomes `parent_session`. `agent_id` and `is_sidechain`
  come from the record, never from the file name.
- **What is counted.** A line without `message.usage` is not a usage record and is not counted in
  any counter. A blank line is ignored. A usage record lands in at most one counter, checked in
  the order `synthetic`, `no_dedupe_key`, `duplicates_removed`. Invalid JSON, invalid UTF-8, or a
  declared field of the wrong type (including a non-object `message`) is `malformed_lines`.
- **Fallbacks.** Dedupe key is `requestId`, else `uuid`; an empty string counts as absent. The
  `uuid` is stored in `Record::request_id` because the model cannot say which one it was (raised
  as `model_change_required`). A record with no `sessionId` takes the parent session (subagent
  file) or the file stem. Cache writes use the 5m/1h breakdown when it reports either figure,
  else `cache_creation_input_tokens` counts as 5m.
- **Raw stamps.** `gitBranch`, `cwd` and `timestamp` are copied untouched.

## Worktree subagents: what `gitBranch` actually says

This is the fact behind #6. When an orchestrator on a feature branch dispatches subagents into
worktrees (`.claude/worktrees/agent-<id>`):

- The subagent's records are filed under the **main checkout's** slug, in
  `<parent-session>/subagents/`, and carry the parent's `sessionId`.
- Their `cwd` is the worktree.
- Their `gitBranch` is **one of two things, inconsistently**:
  - the **orchestrator's** branch. In session `0af124f5`, every record of `agent-a161337e…` carried
    `chore/opponent-deck-resource` even though its cwd was the worktree;
  - `worktree-agent-<id>`, the branch Claude Code creates for the worktree. Sessions `c9a874ed` and
    `ae8c842c` had 30 and 28 such records.
- It is **never the branch the worktree's agent later cut**. `chore/opponent-deck-data`, which that
  worktree actually held, appears on no record.
- The orchestrator's own transcript sometimes carries a few `worktree-agent-<id>` records as well.

What follows for attribution:

- `gitBranch` can't be trusted to name a worktree subagent's branch. The reliable link is
  structural: the file sits under `<parent-session>/subagents/` and the parent's transcript contains
  the dispatch. #6 attributes the whole subagent file to the branch the parent was on at dispatch
  time.
- Resolving `worktree-agent-<id>` through `git worktree list` (what `token_report.py` does) works
  only while the worktree exists. Even then it points at the worktree's **current** branch, which
  pulls the usage away from the feature PR the work was merged into.
- An orchestrator session commonly touches several work branches (session `ae8c842c` touched four),
  so "one session, one branch" doesn't hold (#7).

## Related pages

- [testing.md](testing.md#fixtures): turning these shapes into scrubbed fixtures
- [project_layout.md](project_layout.md): where the parser and the attribution code live
