//! The captures file: hook-captured attribution events in `.tally/captures.jsonl` (decision 0002).
//!
//! A sibling of the ledger, not part of it, so v1 ledger readers are unaffected. One
//! [`CaptureEvent`] per line, in the event's field order, LF line endings, append-only, and set
//! to `merge=union` in `.gitattributes`. An event holds ids and a resolved branch only: no path,
//! no cwd, no content (Governance III), so the [`Redact`](super::Redact) seam, which is typed on
//! ledger entries, is not applied here: there is nothing for it to hide yet. #67 may add one.
//!
//! **Dedupe key:** the whole event. Two lines are duplicates when every field, `ts` included, is
//! equal, which is what a replayed hook produces. A writer skips an event already present, or
//! repeated within the call; a reader collapses duplicates and counts them.
//!
//! **Concurrency:** each call writes all its new lines with one `write_all` on a file opened in
//! append mode (`O_APPEND` on POSIX, `FILE_APPEND_DATA` on Windows), so concurrent hooks do not
//! overwrite one another and a small batch lands contiguously in practice. The duplicate check
//! reads the file first, so two hooks racing on the *same* event can both write it. That is
//! harmless: the reader collapses it and counts it as a duplicate. Nothing is locked.
//!
//! Bad input is counted in [`CaptureReadStats`], never dropped silently and never a panic. Errors
//! name a path, never line content.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::LedgerError;
use crate::model::{CAPTURE_SCHEMA_VERSION, CaptureEvent};

/// Where the captures live, relative to the repository root.
pub const CAPTURES_PATH: &str = ".tally/captures.jsonl";

/// The captures file under the repository root `root`.
#[must_use]
pub fn captures_path(root: &Path) -> PathBuf {
    root.join(CAPTURES_PATH)
}

/// What reading the captures file saw. Every line that is not a usable event is counted here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureReadStats {
    /// Non-blank lines read.
    pub lines: u64,
    /// Distinct events kept.
    pub events: u64,
    /// Lines identical to an event already held.
    pub duplicates: u64,
    /// Lines that are not a valid event (bad UTF-8, bad JSON, missing fields).
    pub malformed: u64,
    /// Lines with a schema version this build does not understand.
    pub unsupported_version: u64,
    /// Lines of a known version whose `kind` this build does not know.
    pub unknown_kind: u64,
    /// 1-based numbers of the malformed, unsupported and unknown-kind lines.
    pub bad_line_numbers: Vec<u64>,
}

/// The captures read from disk: distinct events in first-seen order, plus what was counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Captures {
    events: Vec<CaptureEvent>,
    stats: CaptureReadStats,
}

impl Captures {
    /// The events, deduplicated, in first-seen order.
    #[must_use]
    pub fn events(&self) -> &[CaptureEvent] {
        &self.events
    }

    /// What reading counted.
    #[must_use]
    pub fn stats(&self) -> &CaptureReadStats {
        &self.stats
    }

    /// Whether no event was read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// What an append did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureAppendOutcome {
    /// Events written.
    pub appended: u64,
    /// Events skipped because the file (or an earlier event in the call) already held them.
    pub skipped: u64,
}

/// Reads the captures file under `root`. A missing file is no captures.
///
/// # Errors
/// [`LedgerError::Read`] when the file exists but cannot be read.
pub fn read_captures(root: &Path) -> Result<Captures, LedgerError> {
    read_path(&captures_path(root))
}

/// The kinds this build knows, as they appear on disk.
const KNOWN_KINDS: [&str; 3] = ["session-start", "subagent-stop", "stop"];

/// Why a line is not a usable event.
enum Bad {
    Malformed,
    UnsupportedVersion,
    UnknownKind,
}

/// The dedupe key: the event's serialized line. An event is plain data with string keys, so
/// serialization cannot fail; an empty string would still be deterministic if it somehow did.
fn key(event: &CaptureEvent) -> String {
    serde_json::to_string(event).unwrap_or_default()
}

fn parse_line(bytes: &[u8]) -> Result<CaptureEvent, Bad> {
    let text = std::str::from_utf8(bytes).map_err(|_| Bad::Malformed)?;
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| Bad::Malformed)?;
    match value.get("v").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(CAPTURE_SCHEMA_VERSION) => {}
        Some(_) => return Err(Bad::UnsupportedVersion),
        None => return Err(Bad::Malformed),
    }
    if let Some(kind) = value.get("kind").and_then(serde_json::Value::as_str)
        && !KNOWN_KINDS.contains(&kind)
    {
        return Err(Bad::UnknownKind);
    }
    serde_json::from_value(value).map_err(|_| Bad::Malformed)
}

fn read_path(path: &Path) -> Result<Captures, LedgerError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Captures::default()),
        Err(source) => {
            return Err(LedgerError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let mut out = Captures::default();
    let mut seen: HashSet<String> = HashSet::new();
    for (i, raw) in bytes.split(|b| *b == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        out.stats.lines += 1;
        match parse_line(raw) {
            Ok(event) => {
                if seen.insert(key(&event)) {
                    out.events.push(event);
                } else {
                    out.stats.duplicates += 1;
                }
            }
            Err(bad) => {
                match bad {
                    Bad::Malformed => out.stats.malformed += 1,
                    Bad::UnsupportedVersion => out.stats.unsupported_version += 1,
                    Bad::UnknownKind => out.stats.unknown_kind += 1,
                }
                out.stats.bad_line_numbers.push(i as u64 + 1);
            }
        }
    }
    out.stats.events = out.events.len() as u64;
    Ok(out)
}

/// Appends the events the captures file does not already hold. Creates `.tally/`. Idempotent.
///
/// # Errors
/// [`LedgerError`] when the file cannot be read or written.
pub fn append_captures(
    root: &Path,
    events: &[CaptureEvent],
) -> Result<CaptureAppendOutcome, LedgerError> {
    use std::io::Write;

    let path = captures_path(root);
    let write_err = |source| LedgerError::Write {
        path: path.clone(),
        source,
    };
    let mut seen: HashSet<String> = read_path(&path)?.events.iter().map(key).collect();
    let mut out = CaptureAppendOutcome::default();
    let mut buf = String::new();
    for event in events {
        let line = key(event);
        if seen.contains(&line) {
            out.skipped += 1;
            continue;
        }
        buf.push_str(&line);
        buf.push('\n');
        seen.insert(line);
        out.appended += 1;
    }
    if out.appended == 0 {
        return Ok(out);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(write_err)?;
    }
    if std::fs::read(&path).is_ok_and(|b| !b.is_empty() && !b.ends_with(b"\n")) {
        buf.insert(0, '\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(write_err)?;
    file.write_all(buf.as_bytes()).map_err(write_err)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentId, CaptureKind, Harness, SessionId};

    fn event(session: &str, kind: CaptureKind) -> CaptureEvent {
        CaptureEvent {
            v: CAPTURE_SCHEMA_VERSION,
            harness: Harness::ClaudeCode,
            kind,
            session_id: SessionId::new(session),
            agent_id: Some(AgentId::new("a1")),
            branch: "feat/x".to_owned(),
            worktree_branch: Some("worktree-agent-a1".to_owned()),
            ts: "2026-11-11T10:00:00Z".to_owned(),
        }
    }

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn file(root: &Path) -> Vec<u8> {
        std::fs::read(captures_path(root)).expect("captures file")
    }

    fn seed(root: &Path, body: &str) {
        std::fs::create_dir_all(root.join(".tally")).expect("mkdir");
        std::fs::write(captures_path(root), body).expect("seed");
    }

    #[test]
    fn missing_file_is_no_captures() {
        let dir = root();
        let c = read_captures(dir.path()).unwrap();
        assert!(c.is_empty());
        assert_eq!(c.stats(), &CaptureReadStats::default());
    }

    #[test]
    fn append_creates_dir_writes_lf_lines_and_reads_back() {
        let dir = root();
        let evs = [
            event("s1", CaptureKind::SubagentStop),
            event("s2", CaptureKind::Stop),
        ];
        let out = append_captures(dir.path(), &evs).unwrap();
        assert_eq!((out.appended, out.skipped), (2, 0));
        let bytes = file(dir.path());
        assert!(!bytes.contains(&b'\r') && bytes.ends_with(b"\n"));
        assert_eq!(bytes.split(|b| *b == b'\n').count(), 3);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with(
            r#"{"v":1,"harness":"claude-code","kind":"subagent-stop","session_id":"s1""#
        ));
        let c = read_captures(dir.path()).unwrap();
        assert_eq!(c.events(), &evs);
        assert_eq!((c.stats().lines, c.stats().events), (2, 2));
    }

    #[test]
    fn replayed_event_is_skipped_across_and_within_calls() {
        let dir = root();
        let e = event("s1", CaptureKind::Stop);
        append_captures(dir.path(), std::slice::from_ref(&e)).unwrap();
        let out = append_captures(dir.path(), &[e.clone(), e.clone()]).unwrap();
        assert_eq!((out.appended, out.skipped), (0, 2));
        let mut later = e;
        later.ts = "2026-11-11T11:00:00Z".to_owned();
        let out = append_captures(dir.path(), &[later.clone(), later]).unwrap();
        assert_eq!((out.appended, out.skipped), (1, 1));
        assert_eq!(read_captures(dir.path()).unwrap().events().len(), 2);
    }

    #[test]
    fn append_after_file_without_trailing_newline_starts_a_new_line() {
        let dir = root();
        append_captures(dir.path(), &[event("s1", CaptureKind::Stop)]).unwrap();
        let mut bytes = file(dir.path());
        bytes.pop();
        std::fs::write(captures_path(dir.path()), bytes).unwrap();
        append_captures(dir.path(), &[event("s2", CaptureKind::Stop)]).unwrap();
        let c = read_captures(dir.path()).unwrap();
        assert_eq!(c.events().len(), 2);
        assert_eq!(c.stats().malformed, 0);
    }

    #[test]
    fn reader_collapses_duplicates_and_counts_them() {
        let dir = root();
        let line = serde_json::to_string(&event("s1", CaptureKind::Stop)).unwrap();
        seed(dir.path(), &format!("{line}\r\n\n{line}\n"));
        let c = read_captures(dir.path()).unwrap();
        assert_eq!(c.events().len(), 1);
        assert_eq!(
            (c.stats().lines, c.stats().events, c.stats().duplicates),
            (2, 1, 1)
        );
    }

    #[test]
    fn bad_lines_are_counted_by_kind_with_line_numbers() {
        let dir = root();
        let good = serde_json::to_string(&event("s1", CaptureKind::Stop)).unwrap();
        let body = [
            good.as_str(),
            "not json",
            r#"{"v":2,"harness":"claude-code","kind":"stop"}"#,
            r#"{"v":1,"harness":"claude-code","kind":"teleport","session_id":"s","branch":"b","ts":"t"}"#,
            r#"{"v":1,"kind":"stop"}"#,
        ]
        .join("\n");
        seed(dir.path(), &body);
        let s = read_captures(dir.path()).unwrap().stats().clone();
        assert_eq!(s.lines, 5);
        assert_eq!(s.events, 1);
        assert_eq!(s.malformed, 2);
        assert_eq!(s.unsupported_version, 1);
        assert_eq!(s.unknown_kind, 1);
        assert_eq!(s.bad_line_numbers, vec![2, 3, 4, 5]);
    }

    #[test]
    fn read_error_names_the_path_not_content() {
        let dir = root();
        std::fs::create_dir_all(captures_path(dir.path())).unwrap();
        let err = read_captures(dir.path()).unwrap_err();
        assert!(err.to_string().contains("captures.jsonl"));
    }

    #[test]
    fn concurrent_appends_lose_nothing() {
        let dir = root();
        let p = dir.path().to_owned();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        let e = event(&format!("s{t}-{i}"), CaptureKind::Stop);
                        append_captures(&p, &[e]).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let c = read_captures(&p).unwrap();
        assert_eq!(c.events().len(), 80);
        assert_eq!(c.stats().malformed, 0);
    }
}
