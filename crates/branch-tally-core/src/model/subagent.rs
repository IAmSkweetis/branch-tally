//! Types for rolling a dispatched subagent's usage up to the branch of whoever dispatched it.
//!
//! A source reports raw [`Dispatch`] facts (no branch decision). Attribution turns them into a
//! branch plus [`SubagentTags`] on each [`super::AttributedRecord`]; the summary stage turns those
//! into a [`SubagentSummary`]. Nothing here can hold message content (Governance III).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{AgentId, AttributedRecord, BranchBasis, SessionId};

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

/// Where a subagent ran: the main checkout, or a worktree named from its recorded cwd, so the
/// name survives the worktree being deleted.
///
/// Worktree names compare case-insensitively (AGENTS.md Paths).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workspace {
    /// The repository's main checkout.
    Main,
    /// A worktree, by the name derived from its cwd (its final directory name).
    Worktree(String),
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
        }
    }

    /// Whether this is a worktree rather than the main checkout.
    #[must_use]
    pub fn is_worktree(&self) -> bool {
        matches!(self, Self::Worktree(_))
    }

    fn key(&self) -> (bool, String) {
        match self {
            Self::Main => (false, String::new()),
            Self::Worktree(n) => (true, n.to_lowercase()),
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
    /// Calls summed into this row.
    pub calls: u64,
    /// Tokens summed into this row (same definition as [`super::Bucket::tokens`]).
    pub tokens: u64,
    /// Distinct agent ids behind the row.
    pub agent_ids: BTreeSet<AgentId>,
}

/// The subagent breakdown, precomputed by the summary stage so render never re-derives it.
///
/// `rows` cover subagent records **selected** for the branch. `unattributed_*` count subagent
/// records with no usable dispatch link ([`BranchBasis::SubagentUnlinked`]) across the whole
/// attributed set, since those are normally not selected for the branch at all; use
/// [`SubagentSummary::count_unattributed`] on the attributed records before selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSummary {
    /// Rows in first-seen order.
    pub rows: Vec<SubagentRow>,
    /// Subagent calls with no usable dispatch link.
    pub unattributed_calls: u64,
    /// Tokens of those calls.
    pub unattributed_tokens: u64,
}

impl SubagentSummary {
    /// `(calls, tokens)` of records in `attributed` whose basis is
    /// [`BranchBasis::SubagentUnlinked`].
    #[must_use]
    pub fn count_unattributed(attributed: &[AttributedRecord]) -> (u64, u64) {
        attributed
            .iter()
            .filter(|a| a.basis == BranchBasis::SubagentUnlinked)
            .fold((0, 0), |(c, t), a| (c + 1, t + a.record.usage.tokens()))
    }

    /// Calls across all rows.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.rows.iter().map(|r| r.calls).sum()
    }

    /// Tokens across all rows.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.rows.iter().map(|r| r.tokens).sum()
    }

    /// Distinct dispatched subagents (distinct agent ids) across all rows.
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
        self.rows.is_empty() && self.unattributed_calls == 0
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
            calls,
            tokens,
            agent_ids: ids.iter().map(|i| AgentId::new(*i)).collect(),
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
        }
    }

    #[test]
    fn totals_come_from_the_rows() {
        let s = sample();
        assert_eq!((s.calls(), s.tokens()), (6, 510));
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
