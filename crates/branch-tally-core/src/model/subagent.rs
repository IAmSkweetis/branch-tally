//! Types for rolling a dispatched subagent's usage up to the branch of whoever dispatched it.
//!
//! A source reports raw [`Dispatch`] facts (no branch decision). Attribution turns them into a
//! branch plus [`SubagentTags`] on each [`super::AttributedRecord`]; the summary stage turns those
//! into a [`SubagentSummary`]. Nothing here can hold message content (Governance III).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{AgentId, AttributedRecord, BranchBasis, Bucket, SessionId, saturating_add_flag};

/// One dispatch link a source found: a parent session launched the subagent `agent_id`.
///
/// Raw facts only. `git_branch` is the parent record's stamp at dispatch time, unmodified, and
/// may be missing or itself a `worktree-agent-<id>` stamp; deciding what that means is
/// attribution's job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatch {
    /// The dispatching (parent) session.
    pub session_id: SessionId,
    /// The subagent that was launched.
    pub agent_id: AgentId,
    /// The dispatcher's raw branch stamp at dispatch time.
    pub git_branch: Option<String>,
    /// The dispatch timestamp as a raw ISO-8601 string.
    pub ts: Option<String>,
}

/// Where a subagent ran: the main checkout, a worktree named from its recorded cwd (so the name
/// survives the worktree being deleted), or [`Workspace::Unknown`] when the cwd was missing or
/// outside the repository.
///
/// Worktree names compare case-insensitively (AGENTS.md Paths).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workspace {
    /// The repository's main checkout.
    Main,
    /// A worktree, by the name derived from its cwd (its final directory name).
    Worktree(String),
    /// The recorded cwd was missing or outside the repository, so the workspace cannot be told.
    /// Never the main checkout: the record still rolls up to its dispatcher's branch, but
    /// nothing is claimed about where it ran. It is a workspace for counting, not a worktree.
    Unknown,
}

impl Workspace {
    /// A worktree workspace with the given name.
    #[must_use]
    pub fn worktree(name: impl Into<String>) -> Self {
        Self::Worktree(name.into())
    }

    /// `"main"`, or the worktree name as recorded.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Main => "main".to_owned(),
            Self::Worktree(name) => name.clone(),
            Self::Unknown => "unknown".to_owned(),
        }
    }

    /// The variant as a stable machine-readable string: `"main"`, `"worktree"` or `"unknown"`.
    /// Unlike [`Workspace::label`], a worktree literally named `unknown` cannot be confused
    /// with [`Workspace::Unknown`].
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Worktree(_) => "worktree",
            Self::Unknown => "unknown",
        }
    }

    /// Whether this is a worktree rather than the main checkout.
    #[must_use]
    pub fn is_worktree(&self) -> bool {
        matches!(self, Self::Worktree(_))
    }

    fn key(&self) -> (u8, String) {
        match self {
            Self::Main => (0, String::new()),
            Self::Worktree(n) => (1, n.to_lowercase()),
            Self::Unknown => (2, String::new()),
        }
    }
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Workspace {}

impl Ord for Workspace {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for Workspace {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The two breakdown tags on a subagent record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentTags {
    /// Where it ran.
    pub workspace: Workspace,
    /// Its agent type/name when stamped, else its agent id.
    pub subagent: String,
    /// The agent id, so distinct subagents sharing a name can still be counted apart.
    pub agent_id: AgentId,
}

/// One breakdown row: a (workspace, subagent) pair in first-seen order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentRow {
    /// Where it ran.
    pub workspace: Workspace,
    /// The subagent tag.
    pub subagent: String,
    /// The model id this row is for (`"unknown"` when the record carried none, as in the
    /// per-model table). Rows are keyed by (workspace, subagent, model), so a subagent that
    /// called several models has one row each. Empty on a row serialized before this field
    /// existed: it loads as "model not recorded" (an old multi-model row stays one row whose
    /// `by_model` still holds the split), and render shows a dash.
    #[serde(default)]
    pub model: String,
    /// Calls summed into this row.
    pub calls: u64,
    /// Tokens summed into this row (same definition as [`super::Bucket::tokens`]).
    pub tokens: u64,
    /// Distinct agent ids behind the row.
    pub agent_ids: BTreeSet<AgentId>,
    /// The row's usage split by model id, in first-seen order, so pricing can cost each model at
    /// its own rate. Summary fills it; `calls` and `tokens` equal [`SubagentRow::total`]'s
    /// `calls` and `tokens` whenever this is non-empty. Empty on a summary serialized before
    /// this field existed, in which case `calls`/`tokens` are all there is (and no cost can be
    /// derived for the row).
    #[serde(default)]
    pub by_model: Vec<(String, Bucket)>,
}

impl SubagentRow {
    /// The row's usage summed over its models. Empty `by_model` gives an all-zero bucket.
    #[must_use]
    pub fn total(&self) -> Bucket {
        self.total_checked().0
    }

    /// [`SubagentRow::total`] plus `true` when any field saturated at `u64::MAX`.
    #[must_use]
    pub fn total_checked(&self) -> (Bucket, bool) {
        let mut total = Bucket::default();
        let mut overflowed = false;
        for (_, b) in &self.by_model {
            overflowed |= total.saturating_add_assign(b);
        }
        (total, overflowed)
    }
}

/// The subagent breakdown, precomputed by the summary stage so render never re-derives it.
///
/// `rows` cover only subagent records **selected** for the branch whose dispatch link was
/// verified ([`BranchBasis::DispatcherRolledUp`]); the accessors ([`SubagentSummary::calls`] and
/// friends) therefore count rolled-up records only. `unattributed_*` count subagent records with
/// no usable dispatch link ([`BranchBasis::SubagentUnlinked`]) across the whole attributed set,
/// since those are normally not selected for the branch at all; use
/// [`SubagentSummary::count_unattributed`] on the attributed records before selection.
/// `unlinked_in_total_*` are the subset of those that were selected anyway (via session claim),
/// so they are inside the branch total although their branch is unverified.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSummary {
    /// Rows in first-seen order.
    pub rows: Vec<SubagentRow>,
    /// Subagent calls with no usable dispatch link.
    pub unattributed_calls: u64,
    /// Tokens of those calls.
    pub unattributed_tokens: u64,
    /// Unlinked calls that are included in this report's total (selected via session claim).
    #[serde(default)]
    pub unlinked_in_total_calls: u64,
    /// Tokens of those calls.
    #[serde(default)]
    pub unlinked_in_total_tokens: u64,
}

impl SubagentSummary {
    /// `(calls, tokens)` of records in `attributed` whose basis is
    /// [`BranchBasis::SubagentUnlinked`].
    #[must_use]
    pub fn count_unattributed(attributed: &[AttributedRecord]) -> (u64, u64) {
        attributed
            .iter()
            .filter(|a| a.basis == BranchBasis::SubagentUnlinked)
            .fold((0u64, 0u64), |(c, t), a| {
                (
                    c.saturating_add(1),
                    t.saturating_add(a.record.usage.tokens()),
                )
            })
    }

    /// Calls across all rows (rolled-up records only; unlinked ones are not in any row).
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.rows.iter().fold(0, |t, r| t.saturating_add(r.calls))
    }

    /// Tokens across all rows.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.rows.iter().fold(0, |t, r| t.saturating_add(r.tokens))
    }

    /// `(calls, tokens)` across all rows, plus `true` when either sum saturated at `u64::MAX`.
    #[must_use]
    pub fn totals_checked(&self) -> (u64, u64, bool) {
        let (mut calls, mut tokens) = (0, 0);
        let mut over = false;
        for r in &self.rows {
            over |= saturating_add_flag(&mut calls, r.calls);
            over |= saturating_add_flag(&mut tokens, r.tokens);
        }
        (calls, tokens, over)
    }

    /// Distinct dispatched subagents (distinct agent ids) across all rows, so only subagents whose
    /// dispatch link was verified.
    #[must_use]
    pub fn dispatched(&self) -> u64 {
        let ids: BTreeSet<&AgentId> = self.rows.iter().flat_map(|r| &r.agent_ids).collect();
        ids.len() as u64
    }

    /// Distinct workspaces across all rows.
    #[must_use]
    pub fn workspaces(&self) -> u64 {
        let ws: BTreeSet<&Workspace> = self.rows.iter().map(|r| &r.workspace).collect();
        ws.len() as u64
    }

    /// Distinct worktree workspaces (the main checkout excluded) across all rows.
    #[must_use]
    pub fn worktrees(&self) -> u64 {
        let ws: BTreeSet<&Workspace> = self
            .rows
            .iter()
            .map(|r| &r.workspace)
            .filter(|w| w.is_worktree())
            .collect();
        ws.len() as u64
    }

    /// Whether there is anything to report: rows, or unattributed subagent calls.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.unattributed_calls == 0 && self.unlinked_in_total_calls == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Harness, Record, RequestId, Usage};

    fn row(ws: Workspace, name: &str, calls: u64, tokens: u64, ids: &[&str]) -> SubagentRow {
        SubagentRow {
            workspace: ws,
            subagent: name.to_owned(),
            model: String::new(),
            calls,
            tokens,
            agent_ids: ids.iter().map(|i| AgentId::new(*i)).collect(),
            by_model: Vec::new(),
        }
    }

    fn attributed(basis: BranchBasis, output: u64) -> AttributedRecord {
        let usage = Usage {
            output,
            ..Usage::default()
        };
        AttributedRecord {
            record: Record::new(
                Harness::ClaudeCode,
                SessionId::new("s"),
                RequestId::new("r"),
                usage,
            ),
            branch: "b".to_owned(),
            basis,
            subagent: None,
        }
    }

    fn sample() -> SubagentSummary {
        SubagentSummary {
            rows: vec![
                row(
                    Workspace::worktree("wt-a"),
                    "explorer",
                    3,
                    300,
                    &["a1", "a2"],
                ),
                row(Workspace::worktree("WT-A"), "writer", 2, 200, &["a3"]),
                row(Workspace::Main, "a9", 1, 10, &["a9"]),
            ],
            unattributed_calls: 0,
            unattributed_tokens: 0,
            unlinked_in_total_calls: 0,
            unlinked_in_total_tokens: 0,
        }
    }

    #[test]
    fn totals_come_from_the_rows() {
        let s = sample();
        assert_eq!((s.calls(), s.tokens()), (6, 510));
    }

    #[test]
    fn row_total_saturates_and_flags_overflow() {
        let big = Bucket {
            input: u64::MAX,
            calls: 1,
            ..Bucket::default()
        };
        let mut r = row(Workspace::Main, "a", 1, 1, &["x"]);
        r.by_model = vec![("m".to_owned(), big), ("n".to_owned(), big)];
        let (t, over) = r.total_checked();
        assert!(over);
        assert_eq!((t.input, t.calls), (u64::MAX, 2));
        assert_eq!(r.total().input, u64::MAX);
    }

    #[test]
    fn row_total_of_small_values_does_not_flag() {
        let b = Bucket {
            input: 1,
            calls: 1,
            ..Bucket::default()
        };
        let mut r = row(Workspace::Main, "a", 1, 1, &["x"]);
        r.by_model = vec![("m".to_owned(), b), ("n".to_owned(), b)];
        let want = Bucket {
            input: 2,
            calls: 2,
            ..Bucket::default()
        };
        assert_eq!(r.total_checked(), (want, false));
    }

    #[test]
    fn summary_totals_saturate_and_flag_overflow() {
        let mut s = sample();
        assert_eq!(s.totals_checked(), (6, 510, false));
        s.rows[0].tokens = u64::MAX;
        s.rows[1].calls = u64::MAX;
        let (c, t, over) = s.totals_checked();
        assert!(over);
        assert_eq!((c, t), (u64::MAX, u64::MAX));
        assert_eq!(s.tokens(), u64::MAX);
        assert_eq!(s.calls(), u64::MAX);
    }

    #[test]
    fn unattributed_count_saturates_instead_of_wrapping() {
        let a = vec![
            attributed(BranchBasis::SubagentUnlinked, u64::MAX),
            attributed(BranchBasis::SubagentUnlinked, 5),
        ];
        assert_eq!(SubagentSummary::count_unattributed(&a), (2, u64::MAX));
    }

    #[test]
    fn dispatched_counts_distinct_agent_ids_not_rows() {
        assert_eq!(sample().dispatched(), 4);
    }

    #[test]
    fn workspaces_are_case_insensitive_and_worktrees_exclude_main() {
        let s = sample();
        assert_eq!(s.workspaces(), 2);
        assert_eq!(s.worktrees(), 1);
    }

    #[test]
    fn empty_only_without_rows_and_unattributed() {
        assert!(SubagentSummary::default().is_empty());
        assert!(!sample().is_empty());
        let only_unattributed = SubagentSummary {
            unattributed_calls: 1,
            ..SubagentSummary::default()
        };
        assert!(!only_unattributed.is_empty());
    }

    #[test]
    fn unattributed_counts_only_unlinked_subagent_records() {
        let recs = vec![
            attributed(BranchBasis::SubagentUnlinked, 7),
            attributed(BranchBasis::Stamped, 100),
            attributed(BranchBasis::DispatcherRolledUp, 100),
            attributed(BranchBasis::SubagentUnlinked, 3),
        ];
        assert_eq!(SubagentSummary::count_unattributed(&recs), (2, 10));
    }

    #[test]
    fn workspace_equality_ignores_case_but_not_kind() {
        assert_eq!(Workspace::worktree("A"), Workspace::worktree("a"));
        assert_ne!(Workspace::Main, Workspace::worktree("main"));
        assert_eq!(Workspace::Main.label(), "main");
        assert_eq!(Workspace::worktree("Wt").label(), "Wt");
    }

    #[test]
    fn unknown_workspace_is_its_own_kind_and_counts_as_a_workspace_not_a_worktree() {
        let u = Workspace::Unknown;
        assert_ne!(u, Workspace::Main);
        assert_ne!(u, Workspace::worktree("unknown"));
        assert_eq!(u.label(), "unknown");
        assert_eq!(u.kind(), "unknown");
        assert_eq!(Workspace::Main.kind(), "main");
        assert_eq!(Workspace::worktree("x").kind(), "worktree");
        assert!(!u.is_worktree());
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"unknown\"");
        let s = SubagentSummary {
            rows: vec![
                row(Workspace::Unknown, "a", 1, 5, &["a"]),
                row(Workspace::worktree("w"), "b", 1, 5, &["b"]),
            ],
            ..SubagentSummary::default()
        };
        assert_eq!((s.workspaces(), s.worktrees()), (2, 1));
    }

    #[test]
    fn summaries_serialized_before_the_unlinked_counts_existed_still_load() {
        let old = r#"{"rows":[],"unattributed_calls":2,"unattributed_tokens":9}"#;
        let s: SubagentSummary = serde_json::from_str(old).unwrap();
        assert_eq!((s.unattributed_calls, s.unlinked_in_total_calls), (2, 0));
    }

    #[test]
    fn row_total_sums_models_and_old_rows_without_models_load() {
        let b = |input, output, calls| Bucket {
            input,
            output,
            calls,
            ..Bucket::default()
        };
        let mut r = row(Workspace::Main, "x", 3, 30, &["x"]);
        r.by_model = vec![
            ("m1".to_owned(), b(10, 5, 1)),
            ("m2".to_owned(), b(1, 14, 2)),
        ];
        let t = r.total();
        assert_eq!((t.input, t.output, t.calls, t.tokens()), (11, 19, 3, 30));
        assert_eq!(
            row(Workspace::Main, "y", 1, 1, &[]).total(),
            Bucket::default()
        );
        let old = r#"{"workspace":"main","subagent":"x","calls":1,"tokens":2,"agent_ids":[]}"#;
        let back: SubagentRow = serde_json::from_str(old).unwrap();
        assert_eq!(back.by_model.len(), 0);
    }

    #[test]
    fn dispatch_and_summary_round_trip() {
        let d = Dispatch {
            session_id: SessionId::new("s"),
            agent_id: AgentId::new("a"),
            git_branch: None,
            ts: Some("t".to_owned()),
        };
        let back: Dispatch = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
        let s = sample();
        let back: SubagentSummary =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
