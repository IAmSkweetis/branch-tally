//! Claude Code hook stdin payloads (#13).
//!
//! Only `session_id`, `hook_event_name`, `cwd`, `agent_id` and `agent_type` are deserialized.
//! `cwd` is carried so the caller can resolve a branch with git; it must not be persisted.
//! Field facts and dates: `docs/references/transcript_format.md#hook-payloads`.

use serde::Deserialize;

use crate::model::{AgentId, SessionId};

/// Which hook fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEventKind {
    /// `SessionStart`.
    SessionStart,
    /// `SessionEnd`.
    SessionEnd,
    /// Stop.
    Stop,
    /// `SubagentStart`.
    SubagentStart,
    /// `SubagentStop`.
    SubagentStop,
}

/// A parsed hook payload.
#[derive(Clone, PartialEq, Eq)]
pub struct HookPayload {
    /// The event.
    pub event: HookEventKind,
    /// Session id.
    pub session_id: SessionId,
    /// Agent id.
    pub agent_id: Option<AgentId>,
    /// Agent type.
    pub agent_type: Option<String>,
    /// Cwd.
    pub cwd: Option<String>,
}

/// Manual `Debug`: the working directory's value is never printed (Governance III), only whether
/// it was present.
impl std::fmt::Debug for HookPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookPayload")
            .field("event", &self.event)
            .field("session_id", &self.session_id)
            .field("agent_id", &self.agent_id)
            .field("agent_type", &self.agent_type)
            .field(
                "cwd",
                &self.cwd.as_ref().map(|_| format_args!("<redacted>")),
            )
            .finish()
    }
}

/// Why a payload was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HookError {
    /// Bad JSON.
    #[error("malformed hook payload")]
    Malformed,
    /// Unknown event.
    #[error("unsupported hook event")]
    UnknownEvent,
    /// No session id.
    #[error("hook payload has no session id")]
    MissingSessionId,
    /// No agent id.
    #[error("hook payload has no agent id")]
    MissingAgentId,
}

/// The only fields read. `prompt`, `transcript_path`, `last_assistant_message`, tool input and
/// output, and every other key are not declared, so serde skips them (Governance III).
#[derive(Deserialize)]
struct Raw {
    session_id: Option<String>,
    hook_event_name: Option<String>,
    cwd: Option<String>,
    agent_id: Option<String>,
    agent_type: Option<String>,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.is_empty())
}

/// Parses a hook's stdin JSON into a [`HookPayload`].
///
/// Unknown fields are ignored. An empty `agent_id`, `agent_type` or `cwd` is treated as absent.
/// `agent_id` is required for `SubagentStart` and `SubagentStop`, optional elsewhere (the common
/// fields carry it for hooks that fire inside a subagent).
///
/// # Errors
///
/// [`HookError`] variants carry no payload text: `serde_json`'s own messages can quote values, so
/// they are never surfaced.
pub fn parse_hook_payload(input: &[u8]) -> Result<HookPayload, HookError> {
    let raw: Raw = serde_json::from_slice(input).map_err(|_| HookError::Malformed)?;
    let event = match raw.hook_event_name.as_deref() {
        Some("SessionStart") => HookEventKind::SessionStart,
        Some("SessionEnd") => HookEventKind::SessionEnd,
        Some("Stop") => HookEventKind::Stop,
        Some("SubagentStart") => HookEventKind::SubagentStart,
        Some("SubagentStop") => HookEventKind::SubagentStop,
        _ => return Err(HookError::UnknownEvent),
    };
    let session_id = non_empty(raw.session_id).ok_or(HookError::MissingSessionId)?;
    let agent_id = non_empty(raw.agent_id);
    if agent_id.is_none()
        && matches!(
            event,
            HookEventKind::SubagentStart | HookEventKind::SubagentStop
        )
    {
        return Err(HookError::MissingAgentId);
    }
    Ok(HookPayload {
        event,
        session_id: SessionId::new(session_id),
        agent_id: agent_id.map(AgentId::new),
        agent_type: non_empty(raw.agent_type),
        cwd: non_empty(raw.cwd),
    })
}
