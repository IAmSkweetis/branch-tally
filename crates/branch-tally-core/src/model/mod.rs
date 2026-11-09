//! The normalized types every pipeline stage shares.
//!
//! A [`Record`] is one recorded API call, normalized from a harness transcript. This module holds
//! constructors, accessors and serde derives only. Deciding a branch, a price or a total happens
//! in later stages.
//!
//! Nothing here can hold message content: there is no field for it (Governance III).

use std::fmt;

use serde::{Deserialize, Serialize};

/// An agent harness whose transcripts branch-tally can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Harness {
    /// Anthropic's Claude Code.
    ClaudeCode,
}

impl Harness {
    /// A stable, lowercase, hyphenated name, also used in the ledger and JSON output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
        }
    }
}

impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wraps a raw id exactly as the transcript recorded it.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            /// The raw id.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// A harness session id. For a subagent record this is the id the harness stamped, which for
    /// Claude Code is the parent's session id.
    SessionId
);
string_id!(
    /// The API request id. It is the key duplicates are removed by, so a record without one is
    /// counted as skipped rather than emitted.
    RequestId
);
string_id!(
    /// The id of the subagent that made a call. Absent for the main conversation.
    AgentId
);

/// Token counts for one API call.
///
/// `thinking` is a **share of `output`**, never an addition to it. Sum `output` for the output
/// total and treat `thinking` as informational, or it is counted twice. It is `None` when the
/// transcript did not report it, which is different from reporting zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Uncached input tokens.
    pub input: u64,
    /// Tokens written to the 5-minute cache.
    pub cache_write_5m: u64,
    /// Tokens written to the 1-hour cache.
    pub cache_write_1h: u64,
    /// Tokens read from cache.
    pub cache_read: u64,
    /// Output tokens, including any thinking tokens.
    pub output: u64,
    /// The thinking share of `output`, when reported. Already included in `output`.
    pub thinking: Option<u64>,
}

/// Which skill or named agent the harness says made a call. Both are raw stamps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// The skill active for the call, if stamped.
    pub skill: Option<String>,
    /// The named agent that made the call, if stamped.
    pub agent: Option<String>,
}

/// One recorded API call, normalized from a harness transcript.
///
/// Every `Option` field is `None` only when the transcript genuinely lacked the value. A source
/// never fills in a guess. `git_branch` in particular is the **raw stamp**: choosing the branch a
/// record belongs to is attribution's job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The harness this came from.
    pub harness: Harness,
    /// The session the harness stamped on the call.
    pub session_id: SessionId,
    /// The API request id, the dedupe key.
    pub request_id: RequestId,
    /// The subagent that made the call, if any.
    pub agent_id: Option<AgentId>,
    /// The dispatching session, when the source can tell this call belongs to a subagent file.
    pub parent_session: Option<SessionId>,
    /// The timestamp as an ISO-8601 string, kept raw and not parsed.
    pub ts: Option<String>,
    /// The model id the call was served by.
    pub model: Option<String>,
    /// The working directory as recorded. Raw: it may be `D:\x` or `/d/x` form.
    pub cwd: Option<String>,
    /// The raw git branch stamp. Never rewritten by a source.
    pub git_branch: Option<String>,
    /// Whether the harness marked the call as made by a sidechain (subagent).
    pub is_sidechain: bool,
    /// `true` when `request_id` holds a fallback key (another stable id the harness stamped on
    /// the call) rather than the harness's own request id. The call is still de-duplicated on
    /// it, but a consumer can tell the match was not on a real request id.
    #[serde(default)]
    pub request_id_is_fallback: bool,
    /// The skill or agent stamps.
    pub attribution: Attribution,
    /// Token counts.
    pub usage: Usage,
}

impl Record {
    /// Builds a record from the fields every record must have. Optional fields start as `None`
    /// and `is_sidechain` as `false`; set them on the returned value.
    #[must_use]
    pub fn new(
        harness: Harness,
        session_id: SessionId,
        request_id: RequestId,
        usage: Usage,
    ) -> Self {
        Self {
            harness,
            session_id,
            request_id,
            agent_id: None,
            parent_session: None,
            ts: None,
            model: None,
            cwd: None,
            git_branch: None,
            is_sidechain: false,
            request_id_is_fallback: false,
            attribution: Attribution::default(),
            usage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Record {
        let mut r = Record::new(
            Harness::ClaudeCode,
            SessionId::new("s1"),
            RequestId::new("req_1"),
            Usage {
                input: 1,
                cache_write_5m: 2,
                cache_write_1h: 3,
                cache_read: 4,
                output: 5,
                thinking: Some(2),
            },
        );
        r.git_branch = Some("feat/x".to_owned());
        r.is_sidechain = true;
        r
    }

    #[test]
    fn new_record_does_not_use_a_fallback_key() {
        assert!(!sample().request_id_is_fallback);
    }

    #[test]
    fn fallback_flag_round_trips_and_defaults_when_absent() {
        let mut r = sample();
        r.request_id_is_fallback = true;
        let json = serde_json::to_string(&r).unwrap();
        let back: Record = serde_json::from_str(&json).unwrap();
        assert!(back.request_id_is_fallback);
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("request_id_is_fallback");
        let old: Record = serde_json::from_value(v).unwrap();
        assert!(!old.request_id_is_fallback);
    }

    #[test]
    fn harness_name_is_stable() {
        assert_eq!(Harness::ClaudeCode.as_str(), "claude-code");
        assert_eq!(Harness::ClaudeCode.to_string(), "claude-code");
    }

    #[test]
    fn harness_serializes_as_its_name() {
        let json = serde_json::to_string(&Harness::ClaudeCode).unwrap();
        assert_eq!(json, "\"claude-code\"");
    }

    #[test]
    fn new_record_invents_no_optional_values() {
        let r = Record::new(
            Harness::ClaudeCode,
            SessionId::new("s"),
            RequestId::new("r"),
            Usage::default(),
        );
        assert_eq!(r.agent_id, None);
        assert_eq!(r.parent_session, None);
        assert_eq!(r.ts, None);
        assert_eq!(r.model, None);
        assert_eq!(r.cwd, None);
        assert_eq!(r.git_branch, None);
        assert!(!r.is_sidechain);
        assert_eq!(r.attribution, Attribution::default());
        assert_eq!(r.usage.thinking, None);
    }

    #[test]
    fn ids_keep_the_raw_text() {
        let id = SessionId::new("AbC-1");
        assert_eq!(id.as_str(), "AbC-1");
        assert_eq!(id.to_string(), "AbC-1");
    }

    #[test]
    fn record_round_trips_through_json() {
        let r = sample();
        let json = serde_json::to_string(&r).unwrap();
        let back: Record = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn ids_serialize_as_bare_strings() {
        let json = serde_json::to_string(&RequestId::new("req_9")).unwrap();
        assert_eq!(json, "\"req_9\"");
    }
}
