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
  and the `D:` drive one, so both cwd forms find the same directory. A leading `\\?\` is stripped,
  and `\\?\UNC\srv\share\x` is read as `\\srv\share\x`, slugging to `--srv-share-x` (changed
  2026-11-10; it was `UNC-srv-share-x`). The slug logic lives in core's shared `paths` module
  (`paths::slug_candidates`), not in the Claude Code source, which keeps only the matching rule.
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
- **Raw stamps.** `gitBranch` and `cwd` are copied untouched. `timestamp` is copied untouched too,
  but only when it is strict RFC 3339 (`Timestamp`, #95): anything else (a `|`, a link, an `@name`,
  HTML, control characters, a bad date) leaves the record with no timestamp and is counted in
  `ReadStats::malformed_timestamps`. The record is kept.

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
- A record with no `gitBranch` is recovered from its origin (`OriginRecovered`) only when it was
  read under a **worktree** root. `git worktree list` lists the main checkout too, but its entry
  only says what is checked out now, not what was checked out when the call was made. So an
  unstamped main-checkout record stays `(none)` (#37). The reference recovers it. Fixture:
  `unstamped-main`. Such a record is never dropped silently (#60): the report counts the `(none)`
  records that no branch's report places, repository-wide and before selection, as `unstamped`
  calls and tokens (JSON key `unstamped`, a Markdown footer line). In claim mode a session claim
  places a `(none)` record on the session's only work-shaped branch, so it is not counted: in
  `unstamped-main`, `req_2` is claimed onto `feat/x` and only `req_5` (1 call, 1,115 tokens) is
  unplaced, on any branch's report. Strict mode has no claim, so it counts both (2 calls, 2,221
  tokens). The count appears in live and ledger+live reports, not in ledger-only ones. A subagent
  record with no dispatch link is counted under `subagents.unattributed*` instead, never in both.
  A timeline-ambiguous record that kept `(none)` is in `unstamped` always, and also in the
  session-scoped ambiguous count when its session has a selected record.

### Captured attribution (#13)

`attribute_with_captures` takes the hook captures (`&[CaptureEvent]`) as well; `attribute` is the
same with none. Captured data is authoritative over `git worktree list` and origin recovery, but
not over a real stamp, and the #6 dispatcher roll-up is kept.

- Applies only in claim mode, to a top-level record whose stamp is missing or
  `worktree-agent-<id>`. Strict and timeline mode ignore captures. Subagent records keep the
  roll-up.
- The record's session's captures that **bracket its timestamp** decide: the latest at or before
  and the earliest at or after (all captures at that instant). If they all name one branch, the
  record gets it with `BranchBasis::Captured`, ahead of `WorktreeRecovered` and `OriginRecovered`.
- **Both sides are required.** A record with captures on one side only (for example a
  SessionStart on `A`, then a checkout of `B`, and no Stop yet) is not recovered from captures: the
  user may have switched branches after the last capture. It falls through to the existing
  recovery and stays as uncertain as before.
- Residual limitation: an `A..A` bracket can hide an `A -> B -> A` switch inside the window. Stop
  fires every turn, so the window is one turn, but a captured branch is a bounded-window
  inference, not a certainty. The footnote's captured count says the heuristic was used.
- Disagreement, a record with no parseable timestamp, an unusable captured branch (empty, `HEAD`,
  `worktree-agent-*`) or a session with no captures: nothing is guessed. The existing recovery
  runs, so the record is as uncertain as before.
- The subagent `Workspace` tag is derived from the record's cwd, never from `git worktree list`,
  so it already survives the worktree being removed. A captured worktree branch cannot be stored
  in it without a model field (raised as `model_change_required`).

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

## Hook payloads

Source: the official hooks reference, <https://code.claude.com/docs/en/hooks>, read 2026-11-11.
These are documented shapes, **not** captures from a live session; the Claude Code version they
describe is not stated on the page (it mentions v2.1.251 and v2.1.271 for newer fields). Verify
against a live hook before relying on a field the page marks optional.

A hook command receives one JSON object on stdin. Common fields: `session_id`, `transcript_path`,
`cwd`, `hook_event_name`, plus `permission_mode`, `effort`, `prompt_id`, `scratchpad_dir`, and
`agent_id`/`agent_type` when the hook fires inside a subagent or under `--agent`.

| Event | Extra fields (documented) | Notes |
| ----- | ------------------------- | ----- |
| `SessionStart` | `source` (`startup`, `resume`, `clear`, `compact`, `fork`), optional `model`, `agent_type`, `session_title`; on resume/fork also cache-cost fields | |
| `SessionEnd` | `reason` | 1.5 s default timeout: keep the hook fast. A per-hook `timeout` raises the budget (up to 60 s) only when set in the user's own settings files; timeouts on plugin-provided hooks do not raise it. The env var `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS` (milliseconds) overrides the budget. Verified 2026-11-11 against <https://code.claude.com/docs/en/hooks>. |
| `Stop` | `stop_hook_active`, `last_assistant_message`, `background_tasks`, `session_crons` | `last_assistant_message` is content |
| `SubagentStart` | `agent_id`, `agent_type` | |
| `SubagentStop` | `stop_hook_active`, `agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message`, `background_tasks`, `session_crons` | |
| `WorktreeCreate` | `name` (a slug such as `bold-oak-a3f2`) | A command hook *replaces* worktree creation and prints the path on stdout, so it is not a passive observer |
| `WorktreeRemove` | `worktree_path` (absolute) | Non-zero exit can fail the removal |

Facts for branch-tally:

- **SubagentStop carries `agent_id`** (and `agent_type`, which may be empty for Claude's internal
  agents such as prompt suggestions and `/btw`; those also fire SubagentStop).
- **`session_id` on SubagentStop is the parent session's**, as for the transcript `sessionId`
  stamp, and `transcript_path` is the main transcript while `agent_transcript_path` is
  `<session>/subagents/agent-<id>.jsonl`.
- **Nothing on SubagentStop names the subagent's worktree.** There is no worktree field. The
  only location is `cwd`, which the page says follows Claude (the worktree root once Claude has
  entered one). Whether a subagent's SubagentStop `cwd` is its worktree was **not verified**;
  the page does not say. The caller must resolve a branch from `cwd` with git and treat a `cwd`
  equal to the main checkout as "worktree unknown", not as proof.
- `WorktreeRemove.worktree_path` is the only payload that names a worktree path directly, but a
  hook on it takes over removal semantics, so the core parser does not read it.
- All paths in payloads follow the host: `D:\...` on Windows, possibly `/d/...` under Git Bash.

The parser (`sources/claude_code/hook.rs`, `parse_hook_payload`) deserializes `session_id`,
`hook_event_name`, `cwd`, `agent_id` and `agent_type` only, supports the five events above, and
treats empty strings as absent. `agent_id` is required for SubagentStart/SubagentStop. Errors
(`HookError`) never include payload text, because `serde_json` messages can quote values.
Fixtures: `tests/fixtures/claude-code/hook-payloads/` (hand-written, fake ids and paths).

## Related pages

- [testing.md](testing.md#fixtures): turning these shapes into scrubbed fixtures
- [project_layout.md](project_layout.md): where the parser and the attribution code live
