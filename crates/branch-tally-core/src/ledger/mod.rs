//! The repository ledger: attributed usage kept in `.tally/ledger.jsonl` (decision 0002).
//!
//! One JSON object per line, keyed by the API call's request id and the branch `record` was run
//! for: a request two branches both claim is written once per branch, because each branch's report
//! claims it. The file is append-only and set to `merge=union` in `.gitattributes`, so concurrent
//! branches merge without conflicts; a reader collapses duplicate keys, and a writer appends only
//! keys the ledger does not hold.
//!
//! A line holds usage, model, timestamp, the branch and mode it was recorded for, the assigned
//! branch, session and agent ids, the attribution basis, the selection that kept the record, the
//! skill and agent names and the subagent tags. It never
//! holds message content or a working directory (Governance III): there is no field for either.
//! Every entry passes through a [`Redact`] seam before it is written; [`Identity`] is the default
//! until #9 supplies a policy.
//!
//! Bad input is counted in [`ReadStats`], never dropped silently and never a panic. Errors name
//! a path, never line content.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    AgentId, AttributedRecord, Attribution, BranchBasis, Harness, Mode, Record, RequestId,
    SelectedRecord, Selection, SessionId, SubagentTags, Usage,
};

/// Where the ledger lives, relative to the repository root.
pub const LEDGER_PATH: &str = ".tally/ledger.jsonl";

/// The schema version written in each line's `v` field.
pub const SCHEMA_VERSION: u32 = 1;

/// One ledger line. Field order is the key order on disk, so diffs are stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Schema version.
    pub v: u32,
    /// The API request id, the dedupe key.
    pub request_id: RequestId,
    /// The harness the call came from.
    pub harness: Harness,
    /// The session id the harness stamped.
    pub session_id: SessionId,
    /// The subagent that made the call, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// The timestamp, raw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    /// The model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The branch `record` was run for. With `request_id` it is the ledger key: a request that
    /// two branches both claim is written once per branch.
    pub recorded_for: String,
    /// The mode the record was selected under.
    pub mode: Mode,
    /// The branch attribution assigned. The report's tables use this one.
    pub branch: String,
    /// How the branch was decided.
    pub basis: BranchBasis,
    /// Why the record was kept for the branch.
    pub kept_by: Selection,
    /// The skill stamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// The named-agent stamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The subagent tags, when attribution produced them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<SubagentTags>,
    /// Token counts.
    pub usage: Usage,
    /// When the line was written (RFC 3339 UTC), stamped by [`append`] from its [`Clock`].
    /// Absent on a line written without one, which reads as the oldest. It is bookkeeping, not
    /// usage data: comparing entries for duplicates and conflicts ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
}

/// The clock [`append`] stamps `recorded_at` from. Behind a trait so tests inject a fixed time.
pub trait Clock {
    /// The current time as an RFC 3339 UTC string, `YYYY-MM-DDTHH:MM:SSZ`.
    fn now(&self) -> String;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        format_rfc3339_utc(secs)
    }
}

/// Seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ` (civil-from-days, no dependency).
fn format_rfc3339_utc(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// The ledger key: the request id and the branch it was recorded for.
type EntryKey = (RequestId, String);

impl LedgerEntry {
    fn key(&self) -> EntryKey {
        (self.request_id.clone(), self.recorded_for.clone())
    }

    /// Whether the two entries hold the same data, ignoring `recorded_at`.
    fn same_data(&self, other: &Self) -> bool {
        let mut a = self.clone();
        a.recorded_at.clone_from(&other.recorded_at);
        a == *other
    }

    /// Whether `self` should replace `held` for the same request id: the later `recorded_at`
    /// wins (a missing one is the oldest); a tie falls back to the smaller serialized line.
    fn wins_over(&self, held: &Self) -> bool {
        match self.recorded_at.cmp(&held.recorded_at) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => line_text(self) < line_text(held),
        }
    }

    /// The entry for a selected record. Content, cwd and the raw branch stamp are not carried.
    #[must_use]
    pub fn from_selected(selected: &SelectedRecord, recorded_for: &str, mode: Mode) -> Self {
        let a = &selected.attributed;
        let r = &a.record;
        Self {
            v: SCHEMA_VERSION,
            request_id: r.request_id.clone(),
            harness: r.harness,
            session_id: r.session_id.clone(),
            agent_id: r.agent_id.clone(),
            ts: r.ts.clone(),
            model: r.model.clone(),
            recorded_for: recorded_for.to_owned(),
            mode,
            branch: a.branch.clone(),
            basis: a.basis,
            kept_by: selected.kept_by,
            skill: r.attribution.skill.clone(),
            agent: r.attribution.agent.clone(),
            subagent: a.subagent.clone(),
            usage: r.usage,
            recorded_at: None,
        }
    }

    /// The selected record this entry stands for. Fields the ledger does not store (cwd, raw
    /// branch stamp, origin, parent session) are `None`/default.
    #[must_use]
    pub fn to_selected(&self) -> SelectedRecord {
        let mut record = Record::new(
            self.harness,
            self.session_id.clone(),
            self.request_id.clone(),
            self.usage,
        );
        record.agent_id.clone_from(&self.agent_id);
        record.ts.clone_from(&self.ts);
        record.model.clone_from(&self.model);
        record.attribution = Attribution {
            skill: self.skill.clone(),
            agent: self.agent.clone(),
        };
        SelectedRecord {
            attributed: AttributedRecord {
                record,
                branch: self.branch.clone(),
                basis: self.basis,
                subagent: self.subagent.clone(),
            },
            kept_by: self.kept_by,
        }
    }
}

/// The redaction seam: applied to every entry before it is written.
pub trait Redact {
    /// Returns the entry to write in place of `entry`.
    fn redact(&self, entry: LedgerEntry) -> LedgerEntry;
}

/// The default redaction: writes the entry unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct Identity;

impl Redact for Identity {
    fn redact(&self, entry: LedgerEntry) -> LedgerEntry {
        entry
    }
}

/// What reading a ledger saw. Every line that is not a usable entry is counted here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// Non-blank lines read.
    pub lines: u64,
    /// Distinct request ids kept.
    pub entries: u64,
    /// Lines whose request id was already held with the same data (`recorded_at` aside).
    pub duplicates: u64,
    /// Request ids that appeared with different data; one entry was kept deterministically: the
    /// latest `recorded_at` (a missing one is oldest), then the smallest serialized line.
    pub conflicts: u64,
    /// Lines that are not a valid entry (bad UTF-8, bad JSON, missing fields).
    pub malformed: u64,
    /// Lines with a schema version this build does not understand.
    pub unsupported_version: u64,
    /// 1-based numbers of the malformed and unsupported lines, for locating them.
    pub bad_line_numbers: Vec<u64>,
}

/// A ledger read from disk: deduplicated entries in first-seen order, plus what was counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    entries: Vec<LedgerEntry>,
    stats: ReadStats,
}

impl Ledger {
    /// The entries, one per request id, in first-seen order.
    #[must_use]
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// What reading counted.
    #[must_use]
    pub fn stats(&self) -> &ReadStats {
        &self.stats
    }

    /// Whether the ledger holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether an entry for `id` exists.
    #[must_use]
    pub fn contains(&self, id: &RequestId) -> bool {
        self.entries.iter().any(|e| &e.request_id == id)
    }

    /// The entries as selected records the summary stage can consume.
    #[must_use]
    pub fn to_selected(&self) -> Vec<SelectedRecord> {
        self.entries.iter().map(LedgerEntry::to_selected).collect()
    }

    /// The entries recorded for `branch`, as a ledger of their own (same read stats).
    #[must_use]
    pub fn for_branch(&self, branch: &str) -> Self {
        let entries: Vec<LedgerEntry> = self
            .entries
            .iter()
            .filter(|e| e.recorded_for == branch)
            .cloned()
            .collect();
        Self {
            entries,
            stats: self.stats.clone(),
        }
    }

    /// The distinct modes the entries were recorded under, in strict, claim, timeline order.
    #[must_use]
    pub fn modes(&self) -> Vec<Mode> {
        [Mode::Strict, Mode::Claim, Mode::Timeline]
            .into_iter()
            .filter(|m| self.entries.iter().any(|e| e.mode == *m))
            .collect()
    }
}

/// A ledger operation failed.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// Reading the ledger file failed.
    #[error("cannot read ledger {}: {source}", path.display())]
    Read {
        /// The ledger path.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Writing the ledger failed.
    #[error("cannot write ledger {}: {source}", path.display())]
    Write {
        /// The ledger path.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

fn line_text(entry: &LedgerEntry) -> String {
    // An entry is plain data with string keys, so serialization cannot fail; an empty string
    // would still sort deterministically if it somehow did.
    serde_json::to_string(entry).unwrap_or_default()
}

/// Parses one line: `Ok(entry)`, or `Err(true)` for an unsupported version, `Err(false)` for
/// malformed.
fn parse_line(bytes: &[u8]) -> Result<LedgerEntry, bool> {
    let text = std::str::from_utf8(bytes).map_err(|_| false)?;
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| false)?;
    let version = value.get("v").and_then(serde_json::Value::as_u64);
    match version {
        Some(v) if v == u64::from(SCHEMA_VERSION) => {}
        Some(_) => return Err(true),
        None => return Err(false),
    }
    serde_json::from_value(value).map_err(|_| false)
}

/// Reads the ledger at `path`. A missing file is an empty ledger.
///
/// # Errors
/// [`LedgerError::Read`] when the file exists but cannot be read.
pub fn read(path: &Path) -> Result<Ledger, LedgerError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(source) => {
            return Err(LedgerError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let mut ledger = Ledger::default();
    let mut index: std::collections::HashMap<EntryKey, usize> = std::collections::HashMap::new();
    let mut conflicted: std::collections::HashSet<EntryKey> = std::collections::HashSet::new();
    for (i, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let number = i as u64 + 1;
        ledger.stats.lines += 1;
        let entry = match parse_line(raw) {
            Ok(e) => e,
            Err(unsupported) => {
                if unsupported {
                    ledger.stats.unsupported_version += 1;
                } else {
                    ledger.stats.malformed += 1;
                }
                ledger.stats.bad_line_numbers.push(number);
                continue;
            }
        };
        match index.get(&entry.key()) {
            None => {
                index.insert(entry.key(), ledger.entries.len());
                ledger.entries.push(entry);
            }
            Some(&at) => {
                let held = &mut ledger.entries[at];
                if held.same_data(&entry) {
                    ledger.stats.duplicates += 1;
                } else if conflicted.insert(entry.key()) {
                    ledger.stats.conflicts += 1;
                }
                if entry.wins_over(held) {
                    *held = entry;
                }
            }
        }
    }
    ledger.stats.entries = ledger.entries.len() as u64;
    Ok(ledger)
}

/// What an append did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppendOutcome {
    /// Entries written.
    pub appended: u64,
    /// Records skipped because the ledger (or an earlier record in the call) already held the id.
    pub skipped: u64,
}

/// Appends the records whose request ids the ledger does not hold, after passing each through
/// `redact`. Creates the parent directory. Idempotent: appending the same records again writes
/// nothing. The id compared is the one after redaction, since that is what gets stored.
///
/// # Errors
/// [`LedgerError`] when the ledger cannot be read or written.
pub fn append(
    path: &Path,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    redact: &dyn Redact,
    clock: &dyn Clock,
) -> Result<AppendOutcome, LedgerError> {
    use std::io::Write;

    let write_err = |source| LedgerError::Write {
        path: path.to_owned(),
        source,
    };
    let existing = read(path)?;
    let mut seen: std::collections::HashSet<EntryKey> =
        existing.entries().iter().map(LedgerEntry::key).collect();
    let recorded_at = clock.now();
    let mut out = AppendOutcome::default();
    let mut buf = String::new();
    for rec in records {
        let mut entry = redact.redact(LedgerEntry::from_selected(rec, recorded_for, mode));
        entry.recorded_at = Some(recorded_at.clone());
        if !seen.insert(entry.key()) {
            out.skipped += 1;
            continue;
        }
        buf.push_str(&line_text(&entry));
        buf.push('\n');
        out.appended += 1;
    }
    if out.appended == 0 {
        return Ok(out);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(write_err)?;
    }
    let needs_break = std::fs::read(path).is_ok_and(|b| !b.is_empty() && !b.ends_with(b"\n"));
    if needs_break {
        buf.insert(0, '\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(write_err)?;
    file.write_all(buf.as_bytes()).map_err(write_err)?;
    Ok(out)
}

/// Ledger entries unioned with live records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Union {
    /// One record per request id: the ledger's, then live records the ledger lacks.
    pub records: Vec<SelectedRecord>,
    /// Records only the ledger held.
    pub ledger_only: u64,
    /// Records only the live transcripts held.
    pub live_only: u64,
    /// Request ids both held. The ledger's copy is used, so redaction is not undone.
    pub in_both: u64,
}

/// Unions `ledger` with `live`, deduplicated by request id.
#[must_use]
pub fn union(ledger: &Ledger, live: &[SelectedRecord]) -> Union {
    let mut records = ledger.to_selected();
    let mut seen: std::collections::HashSet<RequestId> = ledger
        .entries()
        .iter()
        .map(|e| e.request_id.clone())
        .collect();
    let (mut live_only, mut in_both) = (0, 0);
    let mut counted_both: std::collections::HashSet<RequestId> = std::collections::HashSet::new();
    for rec in live {
        let id = &rec.attributed.record.request_id;
        if seen.insert(id.clone()) {
            live_only += 1;
            records.push(rec.clone());
        } else if ledger.contains(id) && counted_both.insert(id.clone()) {
            in_both += 1;
        }
    }
    Union {
        ledger_only: ledger.entries().len() as u64 - in_both,
        records,
        live_only,
        in_both,
    }
}

#[cfg(test)]
mod tests;
