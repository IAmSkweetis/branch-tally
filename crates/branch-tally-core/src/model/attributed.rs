//! What attribution hands on: a record plus the branch it was assigned and how.

use serde::{Deserialize, Serialize};

use super::Record;

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
