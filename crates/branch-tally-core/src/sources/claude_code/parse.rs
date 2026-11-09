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

use crate::model::{AgentId, Attribution, Harness, Record, RequestId, SessionId, Usage};
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
    let Some(key) = non_empty(line.request_id).or_else(|| non_empty(line.uuid)) else {
        stats.no_dedupe_key += 1;
        return None;
    };
    if !seen.insert(key.clone()) {
        stats.duplicates_removed += 1;
        return None;
    }

    let session = non_empty(line.session_id).unwrap_or_else(|| ctx.fallback_session.to_owned());
    let mut record = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
        RequestId::new(key),
        raw_usage.into_usage(),
    );
    record.agent_id = non_empty(line.agent_id).map(AgentId::new);
    record.parent_session = ctx.parent_session.map(SessionId::new);
    record.ts = line.timestamp;
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
