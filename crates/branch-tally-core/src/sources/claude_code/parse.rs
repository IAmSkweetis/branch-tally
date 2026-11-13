//! Reading one transcript file into [`Record`]s.
//!
//! The structs below name only the fields branch-tally may read. `message.content` and every
//! other content field are not declared, so serde skips them without storing them
//! (Governance III).

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

use crate::model::{
    AgentId, Attribution, Dispatch, Harness, Origin, Record, RequestId, SessionId, Timestamp, Usage,
};
use crate::sources::{ReadStats, SourceError};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    request_id: Option<String>,
    uuid: Option<String>,
    session_id: Option<String>,
    agent_id: Option<String>,
    is_sidechain: Option<bool>,
    timestamp: Option<String>,
    cwd: Option<String>,
    git_branch: Option<String>,
    attribution_skill: Option<String>,
    attribution_agent: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    model: Option<String>,
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreation>,
    output_tokens_details: Option<OutputDetails>,
}

#[derive(Deserialize)]
struct CacheCreation {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct OutputDetails {
    thinking_tokens: Option<u64>,
}

impl RawUsage {
    /// Cache writes come from the TTL breakdown when it reports either figure; otherwise the flat
    /// `cache_creation_input_tokens` total counts as 5m.
    fn into_usage(self) -> Usage {
        let (short_ttl, long_ttl) = match self.cache_creation {
            Some(CacheCreation {
                ephemeral_5m_input_tokens: five,
                ephemeral_1h_input_tokens: hour,
            }) if five.is_some() || hour.is_some() => {
                (five.unwrap_or_default(), hour.unwrap_or_default())
            }
            _ => (self.cache_creation_input_tokens.unwrap_or_default(), 0),
        };
        Usage {
            input: self.input_tokens.unwrap_or_default(),
            cache_write_5m: short_ttl,
            cache_write_1h: long_ttl,
            cache_read: self.cache_read_input_tokens.unwrap_or_default(),
            output: self.output_tokens.unwrap_or_default(),
            thinking: self.output_tokens_details.and_then(|d| d.thinking_tokens),
        }
    }
}

/// Where a file sits, which supplies the facts the lines themselves may lack.
pub(super) struct FileContext<'a> {
    /// The parent session for a subagent file, taken from the directory structure.
    pub parent_session: Option<&'a str>,
    /// The session id to use when a record carries none: the parent's for a subagent file,
    /// otherwise the file stem.
    pub fallback_session: &'a str,
    /// The transcript root the file was found under, copied onto every record.
    pub origin: &'a Origin,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// Reads one JSONL file, updating `stats` and `seen` and passing kept records to `sink`.
pub(super) fn read_file(
    path: &Path,
    ctx: &FileContext<'_>,
    seen: &mut HashSet<String>,
    stats: &mut ReadStats,
    sink: &mut dyn FnMut(Record),
) -> Result<(), SourceError> {
    let io = |source| SourceError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut reader = BufReader::new(File::open(path).map_err(io)?);
    stats.files_read += 1;

    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(io)? == 0 {
            break;
        }
        let bytes = buf.trim_ascii();
        if bytes.is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_slice::<Line>(bytes) else {
            stats.malformed_lines += 1;
            continue;
        };
        if let Some(record) = to_record(line, ctx, seen, stats) {
            stats.records_emitted += 1;
            sink(record);
        }
    }
    Ok(())
}

fn to_record(
    line: Line,
    ctx: &FileContext<'_>,
    seen: &mut HashSet<String>,
    stats: &mut ReadStats,
) -> Option<Record> {
    // A line without `message.usage` is not a usage record. It is not counted anywhere.
    let message = line.message?;
    let raw_usage = message.usage?;

    if message.model.as_deref().is_some_and(|m| m.starts_with('<')) {
        stats.synthetic += 1;
        return None;
    }
    let (key, is_fallback) = match (non_empty(line.request_id), non_empty(line.uuid)) {
        (Some(request_id), _) => (request_id, false),
        (None, Some(uuid)) => (uuid, true),
        (None, None) => {
            stats.no_dedupe_key += 1;
            return None;
        }
    };
    if !seen.insert(key.clone()) {
        stats.duplicates_removed += 1;
        return None;
    }
    if is_fallback {
        stats.fallback_key_used += 1;
    }

    let session = non_empty(line.session_id).unwrap_or_else(|| ctx.fallback_session.to_owned());
    let mut record = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
        RequestId::new(key),
        raw_usage.into_usage(),
    );
    record.request_id_is_fallback = is_fallback;
    record.agent_id = non_empty(line.agent_id).map(AgentId::new);
    record.parent_session = ctx.parent_session.map(SessionId::new);
    record.origin = Some(ctx.origin.clone());
    // Checked once, here. A timestamp that is not strict RFC 3339 is counted and left off the
    // record: its text is never carried further.
    record.ts = match line.timestamp.as_deref().map(str::parse::<Timestamp>) {
        Some(Ok(ts)) => Some(ts),
        Some(Err(_)) => {
            stats.malformed_timestamps += 1;
            None
        }
        None => None,
    };
    record.model = message.model;
    record.cwd = line.cwd;
    record.git_branch = line.git_branch;
    record.is_sidechain = line.is_sidechain.unwrap_or(false);
    record.attribution = Attribution {
        skill: line.attribution_skill,
        agent: line.attribution_agent,
    };
    Some(record)
}

/// The only part of `toolUseResult` branch-tally reads: the id of a launched subagent.
///
/// `toolUseResult` is a string for most tools and an object for the Agent tool; the object also
/// carries the prompt, description and output path, which are content and are skipped without
/// being stored (Governance III). Anything that is not an object with a string `agentId` is `None`.
#[derive(Default)]
struct LaunchedAgent(Option<String>);

impl<'de> Deserialize<'de> for LaunchedAgent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};

        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LaunchedAgent;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("any value")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut agent = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "agentId" {
                        // A non-string id is not an agent id; its value is dropped, not an error.
                        agent = map.next_value::<StringOrIgnored>()?.0;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(LaunchedAgent(agent))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LaunchedAgent(None))
            }

            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(LaunchedAgent(None))
            }

            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(LaunchedAgent(None))
            }

            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(LaunchedAgent(None))
            }

            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(LaunchedAgent(None))
            }

            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(LaunchedAgent(None))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(LaunchedAgent(None))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// A string kept as is; any other JSON value skipped.
struct StringOrIgnored(Option<String>);

impl<'de> Deserialize<'de> for StringOrIgnored {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};

        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = StringOrIgnored;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("any value")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StringOrIgnored(Some(v.to_owned())))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(StringOrIgnored(None))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(StringOrIgnored(None))
            }

            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(StringOrIgnored(None))
            }

            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(StringOrIgnored(None))
            }

            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(StringOrIgnored(None))
            }

            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(StringOrIgnored(None))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(StringOrIgnored(None))
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatchLine {
    session_id: Option<String>,
    timestamp: Option<String>,
    git_branch: Option<String>,
    #[serde(default)]
    tool_use_result: LaunchedAgent,
}

/// Reads one JSONL file for dispatch links: lines whose `toolUseResult.agentId` names a launched
/// subagent. `seen` holds `(session, agent)` pairs so a replayed line is reported once.
///
/// A line that is not JSON, or has no usable `agentId`, yields nothing and is not counted here:
/// the subagent it would have linked then has no link and is reported as unattributed.
pub(super) fn read_dispatches(
    path: &Path,
    ctx: &FileContext<'_>,
    seen: &mut HashSet<(String, String)>,
    out: &mut Vec<Dispatch>,
) -> Result<(), SourceError> {
    let io = |source| SourceError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut reader = BufReader::new(File::open(path).map_err(io)?);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(io)? == 0 {
            break;
        }
        let bytes = buf.trim_ascii();
        // Most lines are not dispatches; skip parsing them again.
        if bytes.is_empty() || !contains(bytes, b"toolUseResult") {
            continue;
        }
        let Ok(line) = serde_json::from_slice::<DispatchLine>(bytes) else {
            continue;
        };
        let Some(agent) = non_empty(line.tool_use_result.0) else {
            continue;
        };
        let session = non_empty(line.session_id).unwrap_or_else(|| ctx.fallback_session.to_owned());
        if !seen.insert((session.clone(), agent.clone())) {
            continue;
        }
        out.push(Dispatch {
            session_id: SessionId::new(session),
            agent_id: AgentId::new(agent),
            git_branch: non_empty(line.git_branch),
            ts: line.timestamp,
        });
    }
    Ok(())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
