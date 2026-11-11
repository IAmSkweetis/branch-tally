//! A captured attribution event: ids and the resolved branch, written by a harness hook.
//!
//! The hook resolves cwd and worktree to a branch at capture time, so an event holds no path, no
//! cwd and no content (Governance III). There is deliberately no field that could hold either.

use serde::{Deserialize, Serialize};

use super::{AgentId, Harness, SessionId};

/// The schema version written into every [`CaptureEvent`].
pub const CAPTURE_SCHEMA_VERSION: u32 = 1;

/// Which hook moment a [`CaptureEvent`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureKind {
    /// A session started.
    SessionStart,
    /// A subagent finished; its worktree (if any) is about to go away.
    SubagentStop,
    /// A session's main agent stopped.
    Stop,
}

/// One line of `.tally/captures.jsonl`. Field order is the serialization order, so diffs of the
/// committed file stay stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureEvent {
    /// Schema version, [`CAPTURE_SCHEMA_VERSION`] when written.
    pub v: u32,
    /// The harness that fired the hook.
    pub harness: Harness,
    /// Which hook moment this is.
    pub kind: CaptureKind,
    /// The session the event belongs to.
    pub session_id: SessionId,
    /// The subagent, for a [`CaptureKind::SubagentStop`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// The branch the session's checkout was on at the event.
    pub branch: String,
    /// The branch the subagent's worktree held at subagent-stop. `None` when it had no worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    /// When `branch-tally capture` recorded the event: the ledger clock's UTC time at second
    /// precision (`...Z`), not a time carried by the hook payload.
    pub ts: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> CaptureEvent {
        CaptureEvent {
            v: CAPTURE_SCHEMA_VERSION,
            harness: Harness::ClaudeCode,
            kind: CaptureKind::SubagentStop,
            session_id: SessionId::new("s1"),
            agent_id: Some(AgentId::new("a1")),
            branch: "feat/x".to_owned(),
            worktree_branch: Some("worktree-agent-a1".to_owned()),
            ts: "2026-11-11T10:00:00Z".to_owned(),
        }
    }

    #[test]
    fn serializes_with_stable_field_order_and_kebab_case_kind() {
        assert_eq!(
            serde_json::to_string(&event()).unwrap(),
            r#"{"v":1,"harness":"claude-code","kind":"subagent-stop","session_id":"s1","agent_id":"a1","branch":"feat/x","worktree_branch":"worktree-agent-a1","ts":"2026-11-11T10:00:00Z"}"#
        );
    }

    #[test]
    fn round_trips_and_omits_absent_optionals() {
        let mut e = event();
        e.kind = CaptureKind::SessionStart;
        e.agent_id = None;
        e.worktree_branch = None;
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains("agent_id") && !line.contains("worktree_branch"));
        assert!(line.contains(r#""kind":"session-start""#));
        assert_eq!(serde_json::from_str::<CaptureEvent>(&line).unwrap(), e);
    }

    #[test]
    fn stop_kind_serializes_as_stop() {
        assert_eq!(
            serde_json::to_string(&CaptureKind::Stop).unwrap(),
            "\"stop\""
        );
    }
}
