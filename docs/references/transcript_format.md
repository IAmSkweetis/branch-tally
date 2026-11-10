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
  plus `--claude-worktrees` and whatever follows it, usually `-<name>`; the bare prefix is what
  matches (inherited from `token_report.py`). None existed on the machine this page was written
  on.)
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
  the repo slug, and a worktree's when it starts with the slug plus `--claude-worktrees` (as in the
  reference's `transcript_roots`; no trailing `-` or suffix needed), both compared
  case-insensitively. Anything else is excluded, including a sibling repo
  that shares a prefix. A repo path shaped like `/d/x` yields two candidate slugs, the literal one
  and the `D:` drive one, so both cwd forms find the same directory. A leading `\\?\` is stripped.
- **Reading.** `<root>/<session>/subagents/agent-*.jsonl` are subagent files, and the directory
  name becomes `parent_session`. Every other `.jsonl` is read like a main session (see Read
  order). `agent_id` and `is_sidechain`
  come from the record, never from the file name.
- **What is counted.** A line without `message.usage` is not a usage record and is not counted in
  any counter. A blank line is ignored. A usage record lands in at most one counter, checked in
  the order `synthetic`, `no_dedupe_key`, `duplicates_removed`. Invalid JSON, invalid UTF-8, or a
  declared field of the wrong type (including a non-object `message`) is `malformed_lines`.
- **Fallbacks.** Dedupe key is `requestId`, else `uuid`; an empty string counts as absent. The
  `uuid` is stored in `Record::request_id`, with `Record::request_id_is_fallback` set to true, and
  `ReadStats::fallback_key_used` counts the emitted records keyed this way (informational, not
  part of `skipped_total()`; duplicates and skipped lines are not counted), so a report can say how
  many calls were matched by `uuid`. A record with no `sessionId` takes the parent session (subagent
  file) or the file stem. Cache writes use the 5m/1h breakdown when it reports either figure,
  else `cache_creation_input_tokens` counts as 5m.
- **Origin (2026-11-09).** `Record::origin` is `Origin::Main` for records read under the
  repository's own root, including its `subagents/` files, and `Origin::Worktree(<root directory
  name>)` for a worktree sibling root. This matches `origin` in the reference's `load_records`.
- **Read order (2026-11-09, aligned with the reference).** Each root is read in one pass over
  every `.jsonl` at any depth, sorted like Python's `sorted(root.rglob("*.jsonl"))`: `PurePath`
  ordering compares path parts one by one, lower-cased on Windows (`PureWindowsPath`) and
  case-sensitive on POSIX. The source follows the platform it is built for. Roots go in sorted
  order, one dedupe set spans them all, and the first seen wins, so `aaa/subagents/x.jsonl` is read
  before `zzz.jsonl`. Only `<session>/subagents/agent-*.jsonl` gets `parent_session` from its
  directory; every other file, however deeply nested, has no `parent_session` and takes
  `sessionId` from the record, else its file stem. Symlinked directories are not followed.
  Fixture: `read-order`. (Resolves the divergence raised on 2026-11-09.)
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

### Timeline mode (#7)

`attribute_timeline` is a third mode beside strict and claim; claim attribution is unchanged.

- Per session, from the records' own `(timestamp, gitBranch)` pairs. A stamp is a *work branch*
  when the caller's predicate says so (`default_work_branch(default)`: not the repository default
  branch, not `HEAD`, not empty, not `worktree-agent-*`). A `worktree-agent-*` stamp is never work,
  whatever predicate is passed; it is "not a work branch", like `main`.
- A non-work record goes **back** to the last active work branch (`TimelineCarried`); one before the
  session's first work record goes **forward** to it (`TimelineForward`). Work records keep their
  stamp (`Stamped`). A session with no work branch is not attributed: records keep their stamps.
- Undecidable records are `TimelineAmbiguous` and keep the raw stamp: missing or unparseable
  timestamp (RFC 3339 with an explicit offset is required), or the same instant on different work
  branches (the conflict does not move the active branch).
- **No worktree lookup**: no `git worktree list` recovery and no origin-slug recovery. The map is
  used only for the subagent's `Workspace` tag.
- A subagent record gets the dispatcher session's timeline branch **at the dispatch line's
  timestamp** (`DispatcherRolledUp`), not the dispatch line's raw stamp. No link, no usable
  dispatch timestamp, no work branch on the dispatcher, or a conflict at that instant:
  `SubagentUnlinked`, raw stamp kept. Subagent records never move the dispatcher's timeline.
- `select_timeline` keeps a record iff its assigned branch equals the request (`Exact`); there is
  no session claim. An ambiguous record is kept only when its raw stamp equals the request.

## The dispatch link

Observed 2026-11-09, Claude Code 2.1.3xx, by probing real transcripts for ids and key names only:
every `<slug>/<session>/subagents/agent-<id>.jsonl` (33 of 33) is linked from its parent session
file by a `type: "user"` line carrying a `toolUseResult` **object** with `agentId`. The same line
has `gitBranch`, `timestamp`, `sessionId`, `cwd` and `sourceToolAssistantUUID` (the uuid of the
assistant record holding the Agent `tool_use`; its `gitBranch` matched the user line's in 33 of
33). All 33 were async launches (`toolUseResult.isAsync: true`) at spawn depth 1. The line has no
`message.usage`, so it is not a usage record.

- `toolUseResult` also holds `prompt`, `description` and `outputFile`: **content**. The parser
  deserializes `toolUseResult.agentId` only. For other tools `toolUseResult` is usually a string
  or an object without `agentId`; neither is a dispatch.
- The source reports `Dispatch { session_id, agent_id, git_branch, ts }` per line: `git_branch` is
  the line's raw stamp (possibly absent, possibly `worktree-agent-<id>`); deciding what that means
  is attribution's.
- Dispatch lines are also looked for **inside subagent files**, since a subagent could dispatch its
  own subagents (not yet seen in real data; depth 1 only on 2026-11-09). The dispatch session is
  the line's own `sessionId` (the stamped one, i.e. the top-level parent's, in a subagent file),
  else the file's session as for records. Nested dispatch is reported as written and not chased:
  the nested subagent's dispatcher stamp is the first subagent's raw stamp.
- A `(session, agent)` pair seen twice (replayed history) is reported once, first in read order.
- A malformed line, a non-object result, a missing/empty/non-string `agentId`: no dispatch, no
  panic, no `ReadStats` counter. The subagent it would have linked stays unlinked, which attribution
  counts as unattributed. Such unlinked records are never rows in the subagent breakdown; the
  report states how many of them sit inside its total via the session claim.
- `Source::dispatches` has no default body. A wrapper that did not forward it would leave every
  subagent unlinked without any error, so each implementor must provide it.
- `agent-<id>.meta.json` beside each file is not read.

Fixtures: `subagent-*` scenarios; `malformed-and-keyless` holds the non-dispatch shapes.

## Related pages

- [testing.md](testing.md#fixtures): turning these shapes into scrubbed fixtures
- [project_layout.md](project_layout.md): where the parser and the attribution code live
