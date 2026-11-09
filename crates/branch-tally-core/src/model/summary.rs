//! The shape `summary` produces and `render` consumes. Types and accessors only.
//!
//! Tables are `Vec`s in **first-seen order**, not maps. The reference iterates Python dicts
//! (insertion order) and then does stable sorts (`-output`, `-tokens`, `first`), so ties keep
//! first-seen order. A `Vec` lets render reproduce that exactly; a `HashMap` or `BTreeMap` would
//! not. No cost is stored: `render` costs each model's bucket itself (`Bucket.cost(model)`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::SessionId;

/// Summed token counts (`Bucket` in `reference/token_report.py`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    /// Uncached input tokens.
    pub input: u64,
    /// 5-minute cache writes (`w5`).
    pub cache_write_5m: u64,
    /// 1-hour cache writes (`w1h`).
    pub cache_write_1h: u64,
    /// Cache reads (`read`).
    pub cache_read: u64,
    /// Output tokens, thinking included.
    pub output: u64,
    /// Thinking share of `output` (`think`); not added to totals.
    pub thinking: u64,
    /// Number of calls summed.
    pub calls: u64,
}

impl Bucket {
    /// Input + both cache writes + cache read + output; the reference's `tok` in the
    /// attribution table. Thinking is excluded: it is already inside output.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read + self.output
    }
}

/// One session's row (`Session` in the reference). `first`/`last` are raw ISO strings compared
/// as strings, and `""` when a record had no timestamp, exactly as the reference does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRow {
    /// Earliest timestamp seen.
    pub first: String,
    /// Latest timestamp seen.
    pub last: String,
    /// Transcript root of the session's first record: `"main"` or a worktree slug, as text.
    pub origin: String,
    /// Total tokens (the reference's `Session.tokens`).
    pub tokens: u64,
    /// Number of calls.
    pub calls: u64,
    /// Names of subagents seen (the `agent:` suffixes). Sorted by construction.
    pub agents: BTreeSet<String>,
}

impl SessionRow {
    /// A row for the session's first record, with no tokens, calls or agents yet.
    #[must_use]
    pub fn new(first: impl Into<String>, origin: impl Into<String>) -> Self {
        let first = first.into();
        Self {
            last: first.clone(),
            first,
            origin: origin.into(),
            tokens: 0,
            calls: 0,
            agents: BTreeSet::new(),
        }
    }

    /// The reference's `Session.who`: agents joined by `", "`, or an em dash when none.
    #[must_use]
    pub fn who(&self) -> String {
        if self.agents.is_empty() {
            "\u{2014}".to_owned()
        } else {
            self.agents
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

/// The three tables `summarize` returns, each in first-seen order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Per model id (`by_model`).
    pub by_model: Vec<(String, Bucket)>,
    /// Per attribution label such as `main`, `agent:x` or a skill name (`by_attr`).
    pub by_attribution: Vec<(String, Bucket)>,
    /// Per session id (`sessions`).
    pub sessions: Vec<(SessionId, SessionRow)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_tokens_exclude_thinking_and_calls() {
        let b = Bucket {
            input: 1,
            cache_write_5m: 2,
            cache_write_1h: 3,
            cache_read: 4,
            output: 5,
            thinking: 100,
            calls: 100,
        };
        assert_eq!(b.tokens(), 15);
    }

    #[test]
    fn who_is_sorted_and_joined_or_em_dash() {
        let mut s = SessionRow::new("2026-01-01T00:00:00Z", "main");
        assert_eq!(s.who(), "\u{2014}");
        s.agents.insert("zed".to_owned());
        s.agents.insert("alpha".to_owned());
        assert_eq!(s.who(), "alpha, zed");
    }

    #[test]
    fn new_row_starts_with_last_equal_first_and_nothing_counted() {
        let s = SessionRow::new("t", "main");
        assert_eq!((s.first.as_str(), s.last.as_str()), ("t", "t"));
        assert_eq!((s.tokens, s.calls), (0, 0));
    }

    #[test]
    fn summary_round_trips_and_keeps_first_seen_order() {
        let mut sum = Summary::default();
        sum.by_model.push(("z".to_owned(), Bucket::default()));
        sum.by_model.push(("a".to_owned(), Bucket::default()));
        let back: Summary = serde_json::from_str(&serde_json::to_string(&sum).unwrap()).unwrap();
        assert_eq!(back.by_model[0].0, "z");
        assert_eq!(back, sum);
    }
}
