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
//! Hook-captured attribution events live beside the ledger, not in it, in
//! `.tally/captures.jsonl`; see [`append_captures`] and [`read_captures`].
//!
//! Bad input is counted in [`ReadStats`], never dropped silently and never a panic. Errors name
//! a path, never line content.

mod captures;
mod tally_dir;

use std::path::{Path, PathBuf};

pub use tally_dir::{TALLY_DIR, TallyDir};

pub use captures::{
    CAPTURES_PATH, CaptureAppendOutcome, CaptureReadStats, Captures, append_captures,
    captures_path, read_captures,
};
use serde::{Deserialize, Serialize};

use crate::model::{
    AgentId, AttributedRecord, Attribution, BranchBasis, Harness, Mode, Origin, Record, RequestId,
    SelectedRecord, Selection, SessionId, SubagentTags, Usage,
};

/// Where the ledger lives, relative to the repository root.
pub const LEDGER_PATH: &str = ".tally/ledger.jsonl";

/// The ledger's file name inside `.tally/`.
const LEDGER_FILE: &str = "ledger.jsonl";

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
    /// The session's transcript root, stored relative to the repository. Absent on lines written
    /// before it existed, which read as no origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<StoredOrigin>,
    /// Token counts.
    pub usage: Usage,
    /// When the line was written (RFC 3339 UTC), stamped by [`append`] from its [`Clock`].
    /// Absent on a line written without one, which reads as the oldest. It is bookkeeping, not
    /// usage data: comparing entries for duplicates and conflicts ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
}

/// A session origin as the ledger stores it: `"main"`, `{"worktree": "<relative name>"}` or
/// `{"worktree": null}`.
///
/// A transcript slug encodes the worktree's full path, user name included, so it is never
/// stored. The name is the slug minus the main checkout's slug, with leading separators trimmed
/// (`claude-worktrees-agent-abc`). A slug outside the main checkout, or one recorded without
/// knowing the main checkout, is stored unnamed and counted ([`AppendOutcome::unnamed_origins`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoredOrigin {
    /// The main checkout.
    Main,
    /// A worktree: its name relative to the repository, or `None` when it could not be derived.
    Worktree(Option<String>),
}

impl StoredOrigin {
    /// The stored form of `origin`, relative to `main`'s slug.
    fn from_origin(origin: &Origin, main: Option<&Path>) -> Self {
        match origin {
            Origin::Main => Self::Main,
            Origin::Worktree(slug) => Self::Worktree(main.and_then(|m| relative_name(slug, m))),
        }
    }

    /// The origin a report shows, with the name re-prefixed by `main`'s current slug. An unnamed
    /// worktree, or a name with no main checkout to re-prefix it, is `None`: unknown.
    fn to_origin(&self, main: Option<&Path>) -> Option<Origin> {
        match (self, main) {
            (Self::Main, _) => Some(Origin::Main),
            (Self::Worktree(Some(name)), Some(m)) => Some(Origin::worktree(format!(
                "{}--{name}",
                crate::paths::slug(m)
            ))),
            (Self::Worktree(_), _) => None,
        }
    }
}

/// `slug` minus the main checkout's slug and the separators after it. `None` when `slug` is not
/// inside the main checkout (matched case-insensitively on a whole segment) or nothing remains.
fn relative_name(slug: &str, main: &Path) -> Option<String> {
    crate::paths::slug_candidates(main).iter().find_map(|m| {
        let head = slug.get(..m.len())?;
        if !head.eq_ignore_ascii_case(m) {
            return None;
        }
        let rest = slug.get(m.len()..)?;
        if !rest.starts_with('-') {
            return None;
        }
        let name = rest.trim_start_matches('-');
        (!name.is_empty()).then(|| name.to_owned())
    })
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
    pub fn from_selected(
        selected: &SelectedRecord,
        recorded_for: &str,
        mode: Mode,
        main: Option<&Path>,
    ) -> Self {
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
            origin: r
                .origin
                .as_ref()
                .map(|o| StoredOrigin::from_origin(o, main)),
            usage: r.usage,
            recorded_at: None,
        }
    }

    /// The selected record this entry stands for. Fields the ledger does not store (cwd, raw
    /// branch stamp, parent session) are `None`/default. `is_sidechain` is not stored either: it
    /// is derived as `agent_id.is_some()`, because Claude Code sets `agentId` only on subagent
    /// records. That also fixes ledger lines written before this derivation.
    #[must_use]
    pub fn to_selected(&self, main: Option<&Path>) -> SelectedRecord {
        let mut record = Record::new(
            self.harness,
            self.session_id.clone(),
            self.request_id.clone(),
            self.usage,
        );
        record.agent_id.clone_from(&self.agent_id);
        record.is_sidechain = self.agent_id.is_some();
        record.ts.clone_from(&self.ts);
        record.model.clone_from(&self.model);
        record.origin = self.origin.as_ref().and_then(|o| o.to_origin(main));
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
    pub fn to_selected(&self, main: Option<&Path>) -> Vec<SelectedRecord> {
        self.entries.iter().map(|e| e.to_selected(main)).collect()
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

/// Which ledger a report or record operation uses. The default is guarded; an explicit path is
/// the user's own choice and keeps the plain behaviour (#94).
#[derive(Debug, Clone, Copy)]
pub enum LedgerTarget<'a> {
    /// `.tally/ledger.jsonl` at the top level of the checkout being run from (a linked worktree has its own), through [`TallyDir`]: written only when
    /// `.tally/` exists, and a symlink is refused.
    Default,
    /// A path the user gave with `--ledger`. Read and written as is.
    Explicit(&'a Path),
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
    /// A `.tally/` path was refused because it is a symbolic link, not the kind of file expected,
    /// or outside the main checkout (#94). Nothing was read or written through it.
    #[error("refusing to use {}: {reason}", path.display())]
    Refused {
        /// The refused path.
        path: PathBuf,
        /// Why, in words that contain no file content.
        reason: &'static str,
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
    Ok(parse_ledger(&bytes))
}

/// Reads the default ledger, `.tally/ledger.jsonl` under the checkout `root` (the git top level), through the
/// guarded [`TallyDir`] accessor (#94). A missing `.tally/` or file is an empty ledger.
///
/// Use this for the default location; [`read`] is for an explicit `--ledger <path>`.
///
/// # Errors
/// [`LedgerError::Refused`] when `.tally` or the ledger is a symlink or the wrong kind of file;
/// [`LedgerError::Read`] when the file exists but cannot be read.
pub fn read_default(root: &Path) -> Result<Ledger, LedgerError> {
    let Some(tally) = TallyDir::open(root)? else {
        return Ok(Ledger::default());
    };
    Ok(parse_ledger(&tally.read(LEDGER_FILE)?.unwrap_or_default()))
}

fn parse_ledger(bytes: &[u8]) -> Ledger {
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
    ledger
}

/// What an append did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppendOutcome {
    /// Entries written.
    pub appended: u64,
    /// Records skipped because the ledger (or an earlier record in the call) already held the id.
    pub skipped: u64,
    /// Entries written whose worktree origin is unnamed: the session's slug was not inside the
    /// main checkout's, so no name could be stored.
    pub unnamed_origins: u64,
    /// Nothing was written because the repository has no `.tally/` directory: it has not opted
    /// in (#94). Only [`append_default`] sets it. Not an error; the caller can stay quiet.
    pub tally_absent: bool,
}

/// What [`plan_append`] decided: the bytes to add and the outcome.
struct Plan {
    buf: String,
    out: AppendOutcome,
}

/// Works out what to append to a ledger whose current bytes are `existing`.
fn plan_append(
    existing: &[u8],
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    redact: &dyn Redact,
    clock: &dyn Clock,
) -> Plan {
    let held = parse_ledger(existing);
    let mut seen: std::collections::HashSet<EntryKey> =
        held.entries().iter().map(LedgerEntry::key).collect();
    let recorded_at = clock.now();
    let mut out = AppendOutcome::default();
    let mut buf = String::new();
    for rec in records {
        let mut entry = redact.redact(LedgerEntry::from_selected(rec, recorded_for, mode, main));
        entry.recorded_at = Some(recorded_at.clone());
        if !seen.insert(entry.key()) {
            out.skipped += 1;
            continue;
        }
        buf.push_str(&line_text(&entry));
        buf.push('\n');
        out.appended += 1;
        out.unnamed_origins += u64::from(entry.origin == Some(StoredOrigin::Worktree(None)));
    }
    if out.appended > 0 && !existing.is_empty() && !existing.ends_with(b"\n") {
        buf.insert(0, '\n');
    }
    Plan { buf, out }
}

/// Appends the records whose request ids the ledger at `path` does not hold, after passing each
/// through `redact`. Creates the parent directory. Idempotent: appending the same records again
/// writes nothing. The id compared is the one after redaction, since that is what gets stored.
///
/// This is the plain, path-taking form, for an explicit `--ledger <path>` the user chose: it
/// follows symlinks as it always has. The default location goes through [`append_default`].
///
/// # Errors
/// [`LedgerError`] when the ledger cannot be read or written.
pub fn append(
    path: &Path,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    redact: &dyn Redact,
    clock: &dyn Clock,
) -> Result<AppendOutcome, LedgerError> {
    use std::io::Write;

    let write_err = |source| LedgerError::Write {
        path: path.to_owned(),
        source,
    };
    // Read once, for both the dedupe keys and the trailing-newline check. A missing file is empty.
    let existing = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(source) => {
            return Err(LedgerError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let Plan { buf, out } =
        plan_append(&existing, records, recorded_for, mode, main, redact, clock);
    if out.appended == 0 {
        return Ok(out);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(write_err)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(write_err)?;
    file.write_all(buf.as_bytes()).map_err(write_err)?;
    Ok(out)
}

/// Appends to the default ledger, `.tally/ledger.jsonl` under the checkout `root` (the git top level:
/// a linked worktree keeps its own ledger, decision 0002), through
/// the guarded [`TallyDir`] accessor (#94). Writes only when `.tally/` already exists; otherwise
/// nothing is written or created and the outcome has [`AppendOutcome::tally_absent`] set.
/// Idempotent, like [`append`].
///
/// Use this for the default location; [`append`] is for an explicit `--ledger <path>`.
///
/// # Errors
/// [`LedgerError::Refused`] when `.tally` or the ledger is a symlink or the wrong kind of file;
/// [`LedgerError`] otherwise when the ledger cannot be read or written.
pub fn append_default(
    root: &Path,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    redact: &dyn Redact,
    clock: &dyn Clock,
) -> Result<AppendOutcome, LedgerError> {
    let Some(tally) = TallyDir::open(root)? else {
        return Ok(AppendOutcome {
            tally_absent: true,
            ..AppendOutcome::default()
        });
    };
    let existing = tally.read(LEDGER_FILE)?.unwrap_or_default();
    let Plan { buf, out } =
        plan_append(&existing, records, recorded_for, mode, main, redact, clock);
    if out.appended > 0 {
        tally.append(LEDGER_FILE, buf.as_bytes())?;
    }
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
pub fn union(ledger: &Ledger, live: &[SelectedRecord], main: Option<&Path>) -> Union {
    let mut records = ledger.to_selected(main);
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
