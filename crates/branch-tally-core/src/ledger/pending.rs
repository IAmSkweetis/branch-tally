//! Resolving pending ledger lines (decision 0006).
//!
//! `record` on the default branch writes each line `pending`: recorded on the default branch and not
//! yet resolved to a branch. A pending line is never edited. When evidence shows which branch was cut next, a
//! **resolution line** is appended: a copy of the pending line recorded for that branch, naming
//! the default branch it came from. This module is the pure part: it takes the ledger and the
//! evidence the caller gathered and returns the lines to append plus a count of what stayed
//! pending and why (Governance II). It reads no file, runs no git and holds no salt: the lines it
//! copies are already hashed, and the evidence is keyed by the same (hashed) request ids.

use std::collections::{HashMap, HashSet};

use super::{Ledger, LedgerEntry, Resolved};
use crate::model::{AttributedRecord, BranchBasis, Bucket, Pending, RequestId, Selection};

/// What the timeline said about one pending request, from `attribute_timeline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvidence {
    /// The branch the timeline assigned.
    pub branch: String,
    /// How it was decided. Only [`BranchBasis::TimelineForward`] and
    /// [`BranchBasis::DispatcherRolledUp`] resolve a line.
    pub basis: BranchBasis,
}

impl TimelineEvidence {
    /// The evidence an attributed record from `attribute_timeline` stands for.
    #[must_use]
    pub fn from_attributed(attributed: &AttributedRecord) -> Self {
        Self {
            branch: attributed.branch.clone(),
            basis: attributed.basis,
        }
    }
}

/// Everything known about one pending request. Both sides are optional: the live transcripts may
/// no longer exist, and a session may have no capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingEvidence {
    /// The timeline over the live records, with ids hashed as the ledger's are.
    pub timeline: Option<TimelineEvidence>,
    /// The work branch of the session's first capture at or after the record's `ts`.
    pub capture: Option<String>,
}

/// Evidence by (hashed) request id. A pending line with no entry here has no evidence: its
/// transcript is gone, or no salt matching the line's fingerprint was available.
pub type EvidenceMap = HashMap<RequestId, PendingEvidence>;

/// What resolution did, counted by reason. Every pending line lands in exactly one field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingStats {
    /// Resolution lines produced.
    pub resolved: u64,
    /// Already covered: another branch holds a non-pending line for the request, or the request
    /// is already resolved to the target. Nothing is appended.
    pub covered: u64,
    /// Evidence points at a branch other than the one being resolved: left for that branch.
    pub other_branch: u64,
    /// Stays pending: the line has no `ts`.
    pub no_timestamp: u64,
    /// Stays pending: no evidence for the request (no transcript, or no salt matched).
    pub no_evidence: u64,
    /// Stays pending: the timeline cannot place it (same-instant records on conflicting work
    /// branches).
    pub ambiguous: u64,
    /// Stays pending: the session never reached a work branch.
    pub no_work_branch: u64,
    /// Stays pending: a subagent with no usable dispatch link.
    pub subagent_unlinked: u64,
    /// Stays pending: the session returned to the default branch after a work branch, so the call
    /// is never assigned back to that earlier branch.
    pub carried_back: u64,
    /// Stays pending: transcripts and captures name different branches.
    pub disagreement: u64,
    /// Stays pending: the request is already resolved to a different branch.
    pub resolved_elsewhere: u64,
}

impl PendingStats {
    /// Lines that stay pending because they could not be placed with certainty. The caller must
    /// report these as uncertain.
    #[must_use]
    pub fn uncertain(&self) -> u64 {
        self.no_timestamp
            + self.no_evidence
            + self.ambiguous
            + self.no_work_branch
            + self.subagent_unlinked
            + self.carried_back
            + self.disagreement
            + self.resolved_elsewhere
    }
}

/// The result of [`resolve_pending`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    /// Lines to append, one per resolved request, in ledger order.
    pub entries: Vec<LedgerEntry>,
    /// What happened to every pending line.
    pub stats: PendingStats,
}

/// Resolves the ledger's unresolved pending lines that belong to `target`.
///
/// `recorded_at` stamps the new lines (the caller's clock). Nothing is appended for a line
/// that is already covered, or that evidence places on another branch; a line that cannot be
/// placed stays pending and is counted by reason in [`Resolution::stats`].
#[must_use]
pub fn resolve_pending(
    ledger: &Ledger,
    target: &str,
    evidence: &EvidenceMap,
    recorded_at: &str,
) -> Resolution {
    let mut out = Resolution::default();
    let mut seen: HashSet<&RequestId> = HashSet::new();
    for pending in ledger.entries().iter().filter(|e| e.pending) {
        if !seen.insert(&pending.request_id) {
            continue;
        }
        let stats = &mut out.stats;
        match settle(ledger, pending, target, evidence.get(&pending.request_id)) {
            Settled::Covered => stats.covered += 1,
            Settled::OtherBranch => stats.other_branch += 1,
            Settled::Stays(reason) => reason.count(stats),
            Settled::Resolve { basis } => {
                stats.resolved += 1;
                out.entries
                    .push(resolution_of(pending, target, basis, recorded_at));
            }
        }
    }
    out
}

/// Why a pending line stays pending.
#[derive(Debug, Clone, Copy)]
enum Stay {
    NoTimestamp,
    NoEvidence,
    Ambiguous,
    NoWorkBranch,
    SubagentUnlinked,
    CarriedBack,
    Disagreement,
    ResolvedElsewhere,
}

impl Stay {
    fn count(self, stats: &mut PendingStats) {
        let field = match self {
            Self::NoTimestamp => &mut stats.no_timestamp,
            Self::NoEvidence => &mut stats.no_evidence,
            Self::Ambiguous => &mut stats.ambiguous,
            Self::NoWorkBranch => &mut stats.no_work_branch,
            Self::SubagentUnlinked => &mut stats.subagent_unlinked,
            Self::CarriedBack => &mut stats.carried_back,
            Self::Disagreement => &mut stats.disagreement,
            Self::ResolvedElsewhere => &mut stats.resolved_elsewhere,
        };
        *field += 1;
    }
}

/// What to do with one pending line.
enum Settled {
    Covered,
    OtherBranch,
    Stays(Stay),
    Resolve { basis: BranchBasis },
}

/// The non-pending lines for the same request on another branch than the pending line's own.
fn other_lines<'a>(
    ledger: &'a Ledger,
    pending: &'a LedgerEntry,
) -> impl Iterator<Item = &'a LedgerEntry> {
    ledger.entries().iter().filter(move |e| {
        !e.pending && e.request_id == pending.request_id && e.recorded_for != pending.recorded_for
    })
}

fn settle(
    ledger: &Ledger,
    pending: &LedgerEntry,
    target: &str,
    evidence: Option<&PendingEvidence>,
) -> Settled {
    // An ordinary line for the request on another branch (or a resolution to the target) already
    // claims it. A resolution to a different branch settles it elsewhere.
    let mut elsewhere = false;
    for other in other_lines(ledger, pending) {
        match &other.resolved {
            Some(_) if other.recorded_for != target => elsewhere = true,
            _ => return Settled::Covered,
        }
    }
    if elsewhere {
        return Settled::Stays(Stay::ResolvedElsewhere);
    }
    if pending.ts.is_none() {
        return Settled::Stays(Stay::NoTimestamp);
    }
    let Some(evidence) = evidence else {
        return Settled::Stays(Stay::NoEvidence);
    };
    let from_timeline = match &evidence.timeline {
        None => None,
        Some(t) => match t.basis {
            BranchBasis::TimelineForward | BranchBasis::DispatcherRolledUp => {
                Some((t.branch.as_str(), t.basis))
            }
            BranchBasis::TimelineAmbiguous => return Settled::Stays(Stay::Ambiguous),
            BranchBasis::TimelineCarried => return Settled::Stays(Stay::CarriedBack),
            BranchBasis::SubagentUnlinked => return Settled::Stays(Stay::SubagentUnlinked),
            // The session never reached a work branch: only a capture can still place it.
            BranchBasis::Stamped if evidence.capture.is_none() => {
                return Settled::Stays(Stay::NoWorkBranch);
            }
            BranchBasis::Stamped => None,
            _ => return Settled::Stays(Stay::NoEvidence),
        },
    };
    let (branch, basis) = match (from_timeline, evidence.capture.as_deref()) {
        (Some((b, _)), Some(c)) if b != c => return Settled::Stays(Stay::Disagreement),
        (Some(found), _) => found,
        (None, Some(c)) => (c, BranchBasis::Captured),
        (None, None) => return Settled::Stays(Stay::NoEvidence),
    };
    if branch != target || branch == pending.recorded_for {
        return Settled::OtherBranch;
    }
    Settled::Resolve { basis }
}

/// The resolution line for `pending`: a copy recorded for `target`.
fn resolution_of(
    pending: &LedgerEntry,
    target: &str,
    basis: BranchBasis,
    recorded_at: &str,
) -> LedgerEntry {
    let mut entry = pending.clone();
    entry.resolved = Some(Resolved {
        from: pending.recorded_for.clone(),
    });
    entry.pending = false;
    target.clone_into(&mut entry.recorded_for);
    target.clone_into(&mut entry.branch);
    entry.basis = basis;
    entry.kept_by = Selection::Exact;
    entry.recorded_at = Some(recorded_at.to_owned());
    entry
}

impl Ledger {
    /// The pending lines still unresolved: no other branch holds a non-pending line for the
    /// request (a resolution or an ordinary claim). Deduplicated by request id.
    #[must_use]
    pub fn unresolved_pending(&self) -> Pending {
        let mut seen: HashSet<&RequestId> = HashSet::new();
        let mut out = Pending::default();
        for p in self.entries().iter().filter(|e| e.pending) {
            if other_lines(self, p).next().is_some() || !seen.insert(&p.request_id) {
                continue;
            }
            let u = &p.usage;
            let bucket = Bucket {
                input: u.input,
                cache_write_5m: u.cache_write_5m,
                cache_write_1h: u.cache_write_1h,
                cache_read: u.cache_read,
                output: u.output,
                ..Bucket::default()
            };
            out.lines += 1;
            out.calls += 1;
            out.tokens = out.tokens.saturating_add(bucket.tokens());
        }
        out
    }
}

#[cfg(test)]
mod tests;
