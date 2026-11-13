//! The pure check behind `branch-tally ledger check` (#96): given the committed `.tally/` files
//! on the base ref and on the head, list every way the head breaks the rules. A violation names
//! the file, the 1-based line number in the head and the rule, never the line text (Governance
//! III).

use std::collections::{HashMap, HashSet};

use super::captures::parse_event;
use super::{EntryKey, parse_line};
use crate::model::Timestamp;

/// A single token field above this on one ledger line is a violation.
pub const MAX_TOKENS_PER_FIELD: u64 = 10_000_000;

/// Which committed file a violation is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedFile {
    /// `.tally/ledger.jsonl`.
    Ledger,
    /// `.tally/captures.jsonl`.
    Captures,
}

/// The rule a line or file broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// A line present on base is missing from the head (or changed). No head line number.
    BaseLineRemoved,
    /// A head-only line does not parse.
    Unparseable,
    /// A new ledger line's `recorded_for` is not the head branch.
    WrongBranch,
    /// A new ledger line has no `recorded_at`.
    MissingRecordedAt,
    /// A timestamp on a new line is not valid RFC 3339.
    InvalidTimestamp,
    /// A timestamp on a new line is after `now`.
    FutureTimestamp,
    /// A new ledger line's `(request_id, recorded_for)` already exists on base.
    DuplicateKey,
    /// A new ledger line has a token field above [`MAX_TOKENS_PER_FIELD`].
    TokenOverCeiling {
        /// The usage field's name.
        field: &'static str,
    },
}

/// One broken rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Violation {
    /// The file.
    pub file: CheckedFile,
    /// 1-based line number in the head; `None` for a removed base line.
    pub line: Option<u64>,
    /// The rule.
    pub rule: Rule,
}

/// What to check. A `None` file does not exist on that side.
#[derive(Debug, Clone, Copy)]
pub struct CheckInput<'a> {
    /// `.tally/ledger.jsonl` on the base ref.
    pub base_ledger: Option<&'a [u8]>,
    /// `.tally/ledger.jsonl` on the head.
    pub head_ledger: Option<&'a [u8]>,
    /// `.tally/captures.jsonl` on the base ref.
    pub base_captures: Option<&'a [u8]>,
    /// `.tally/captures.jsonl` on the head.
    pub head_captures: Option<&'a [u8]>,
    /// The branch the head is.
    pub head_branch: &'a str,
    /// The current time, from the caller's clock.
    pub now: &'a Timestamp,
}

/// Every violation found; empty means clean.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Violations, ledger first, in head line order.
    pub violations: Vec<Violation>,
}

impl CheckReport {
    /// Whether nothing was violated.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Checks the head's `.tally/` files against the base's.
///
/// Lines are compared byte for byte (a trailing `\r` aside), as a multiset: the head may hold
/// the base's lines plus new ones in any order. Blank lines are ignored but still count towards
/// line numbers.
#[must_use]
pub fn check(input: &CheckInput<'_>) -> CheckReport {
    let mut violations = Vec::new();

    let ledger = split(input.base_ledger, input.head_ledger);
    let base_keys: HashSet<EntryKey> = ledger
        .base_all
        .iter()
        .filter_map(|l| parse_line(l).ok())
        .map(|(e, _)| e.key())
        .collect();
    for (number, raw) in &ledger.new_lines {
        check_ledger_line(raw, *number, input, &base_keys, &mut violations);
    }
    removed(CheckedFile::Ledger, ledger.removed, &mut violations);

    let captures = split(input.base_captures, input.head_captures);
    for (number, raw) in &captures.new_lines {
        check_capture_line(raw, *number, input.now, &mut violations);
    }
    removed(CheckedFile::Captures, captures.removed, &mut violations);

    CheckReport { violations }
}

/// One file's lines, split into base-only and head-only by multiset difference.
struct Split<'a> {
    base_all: Vec<&'a [u8]>,
    /// Head lines the base does not hold, with their 1-based head line numbers.
    new_lines: Vec<(u64, &'a [u8])>,
    /// How many base lines the head no longer holds.
    removed: u64,
}

fn content_lines(bytes: Option<&[u8]>) -> Vec<(u64, &[u8])> {
    bytes
        .unwrap_or_default()
        .split(|b| *b == b'\n')
        .enumerate()
        .map(|(i, raw)| (i as u64 + 1, raw.strip_suffix(b"\r").unwrap_or(raw)))
        .filter(|(_, raw)| !raw.iter().all(u8::is_ascii_whitespace))
        .collect()
}

fn split<'a>(base: Option<&'a [u8]>, head: Option<&'a [u8]>) -> Split<'a> {
    let base_all: Vec<&[u8]> = content_lines(base).into_iter().map(|(_, l)| l).collect();
    let mut unmatched: HashMap<&[u8], u64> = HashMap::new();
    for line in &base_all {
        *unmatched.entry(line).or_default() += 1;
    }
    let mut new_lines = Vec::new();
    for (number, line) in content_lines(head) {
        match unmatched.get_mut(line) {
            Some(n) if *n > 0 => *n -= 1,
            _ => new_lines.push((number, line)),
        }
    }
    let removed = unmatched.values().sum();
    Split {
        base_all,
        new_lines,
        removed,
    }
}

fn removed(file: CheckedFile, count: u64, out: &mut Vec<Violation>) {
    for _ in 0..count {
        out.push(Violation {
            file,
            line: None,
            rule: Rule::BaseLineRemoved,
        });
    }
}

/// Which way a timestamp failed, if it did.
fn timestamp_rule(text: &str, now: &Timestamp) -> Option<Rule> {
    match text.parse::<Timestamp>() {
        Err(_) => Some(Rule::InvalidTimestamp),
        Ok(ts) if ts.instant() > now.instant() => Some(Rule::FutureTimestamp),
        Ok(_) => None,
    }
}

fn check_ledger_line(
    raw: &[u8],
    line: u64,
    input: &CheckInput<'_>,
    base_keys: &HashSet<EntryKey>,
    out: &mut Vec<Violation>,
) {
    let mut flag = |rule| {
        out.push(Violation {
            file: CheckedFile::Ledger,
            line: Some(line),
            rule,
        });
    };
    // A line whose `ts` the reader would silently clear is not clean either.
    let Ok((entry, false)) = parse_line(raw) else {
        flag(Rule::Unparseable);
        return;
    };
    if entry.recorded_for != input.head_branch {
        flag(Rule::WrongBranch);
    }
    match &entry.recorded_at {
        None => flag(Rule::MissingRecordedAt),
        Some(text) => {
            if let Some(rule) = timestamp_rule(text, input.now) {
                flag(rule);
            }
        }
    }
    if base_keys.contains(&entry.key()) {
        flag(Rule::DuplicateKey);
    }
    let u = &entry.usage;
    let fields = [
        ("input", Some(u.input)),
        ("cache_write_5m", Some(u.cache_write_5m)),
        ("cache_write_1h", Some(u.cache_write_1h)),
        ("cache_read", Some(u.cache_read)),
        ("output", Some(u.output)),
        ("thinking", u.thinking),
    ];
    for (field, value) in fields {
        if value.is_some_and(|n| n > MAX_TOKENS_PER_FIELD) {
            flag(Rule::TokenOverCeiling { field });
        }
    }
}

fn check_capture_line(raw: &[u8], line: u64, now: &Timestamp, out: &mut Vec<Violation>) {
    let rule = match parse_event(raw) {
        None => Some(Rule::Unparseable),
        Some(event) => timestamp_rule(&event.ts, now),
    };
    if let Some(rule) = rule {
        out.push(Violation {
            file: CheckedFile::Captures,
            line: Some(line),
            rule,
        });
    }
}
