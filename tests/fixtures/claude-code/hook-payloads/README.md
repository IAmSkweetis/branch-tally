# hook-payloads

Hand-written synthetic Claude Code hook stdin payloads (issue #13). Every id and path is fake, and
`PLACEHOLDER` stands in for content. Nothing here comes from a real session.

Shapes follow the official hook docs (<https://code.claude.com/docs/en/hooks>, read 2026-11-11).
The exact Claude Code patch version was not observed: these are documented shapes, not captures.

| File | Reproduces | Correct result |
| ---- | ---------- | -------------- |
| `session-start.json` | SessionStart, `/d/repo` cwd, unknown fields | event `SessionStart`, no agent |
| `subagent-stop-windows.json` | SubagentStop, `D:\` worktree cwd | agent `fake1`, type `Explore` |
| `subagent-stop-posix.json` | SubagentStop, `/d/` cwd, empty `agent_type` | agent `fake2`, type none |
| `stop.json`, `session-end.json` | Stop, SessionEnd | session only |
| `unknown-event.json` | PreToolUse | `UnknownEvent` |
| `missing-session.json` | no `session_id` | `MissingSessionId` |
| `subagent-stop-no-agent.json` | SubagentStop without `agent_id` | `MissingAgentId` |
| `wrong-type.json` | `cwd` is a number | `Malformed` |

Malformed JSON and empty input are tested inline in the test file.
