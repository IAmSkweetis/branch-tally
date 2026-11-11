//! What attribution hands on: a record plus the branch it was assigned and how.

use serde::{Deserialize, Serialize};

use super::{Record, SubagentTags};

/// How an [`AttributedRecord`]'s branch was decided. Governance II: a recovered branch is a
/// heuristic and the report must be able to say so.
///
/// Mirrors the three outcomes in `load_records` of `reference/token_report.py`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BranchBasis {
    /// The branch is the raw stamp (including the literal `(none)` the reference uses for a
    /// missing stamp), or strict mode forbade recovery.
    Stamped,
    /// A `worktree-agent-<id>` stamp was replaced by the branch git says that worktree holds.
    WorktreeRecovered,
    /// A missing stamp was replaced by the branch the record's transcript-root worktree holds.
    OriginRecovered,
    /// A dispatched subagent's record, assigned the branch its dispatcher was on at dispatch
    /// time (the parent record's stamp), whatever the subagent's own stamp said.
    DispatcherRolledUp,
    /// A subagent record with no usable dispatch link (no link found, or the dispatcher's stamp
    /// was missing or itself a `worktree-agent-<id>`). It keeps its raw stamp and is **counted
    /// as unattributed**, never guessed.
    SubagentUnlinked,
    /// Timeline mode: a non-work record before the session's first work branch, assigned
    /// **forward** to that first work branch.
    TimelineForward,
    /// Timeline mode: a non-work record after a work branch was active, carried **back** to the
    /// last active work branch.
    TimelineCarried,
    /// Timeline mode: the record's place on the timeline cannot be decided with certainty (no
    /// parseable timestamp, or same-timestamp records on conflicting work branches). It keeps
    /// its raw stamp and is **counted as ambiguous**, never guessed.
    TimelineAmbiguous,
    /// The branch came from a captured hook event (`.tally/captures.jsonl`) rather than from the
    /// transcript stamp or `git worktree list`: a captured session or subagent-stop event
    /// recovered a missing or `worktree-agent-<id>` stamp. Captured data is authoritative, but
    /// the report must say it was used (Governance II).
    Captured,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_basis_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&BranchBasis::Captured).unwrap(),
            "\"captured\""
        );
    }

    #[test]
    fn timeline_bases_serialize_kebab_case() {
        let names: Vec<String> = [
            BranchBasis::TimelineForward,
            BranchBasis::TimelineCarried,
            BranchBasis::TimelineAmbiguous,
        ]
        .iter()
        .map(|b| serde_json::to_string(b).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                "\"timeline-forward\"",
                "\"timeline-carried\"",
                "\"timeline-ambiguous\""
            ]
        );
    }
}

/// A record with the branch attribution assigned to it.
///
/// `record.git_branch` stays the raw stamp; `branch` is the decided value (`(none)` when the
/// transcript had no stamp and nothing recovered one, as in the reference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributedRecord {
    /// The record as the source produced it.
    pub record: Record,
    /// The branch assigned after worktree recovery (`load_records`' `branch`).
    pub branch: String,
    /// How `branch` was decided.
    pub basis: BranchBasis,
    /// Workspace and subagent tags. `Some` for every claim-mode record from a subagent file that
    /// carries an agent id (rolled up or unlinked; a missing or outside-repo cwd is
    /// [`super::Workspace::Unknown`]). `None` for other records, and in strict mode, which keeps
    /// raw stamps. Absent in older serialized data.
    #[serde(default)]
    pub subagent: Option<SubagentTags>,
}

/// Why a record was kept for the requested branch (`select` in the reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Selection {
    /// Its own branch equals the requested branch.
    Exact,
    /// Kept only because its session was claimed by the branch (session-claim heuristic): the
    /// record itself carries another branch. This is what the report counts as recovered.
    SessionClaim,
}

/// An attributed record kept for the requested branch, with the reason it was kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedRecord {
    /// The attributed record.
    pub attributed: AttributedRecord,
    /// Why it was kept.
    pub kept_by: Selection,
}
