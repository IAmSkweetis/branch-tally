//! Pending ledger lines at the report layer (decision 0006).
//!
//! `record` on the default branch writes pending lines; `record` on any other branch, and the
//! explicit [`resolve_at`], resolve the ones that target that branch by appending. This module
//! gathers the evidence (the timeline over the live records, and the captures), hands it to
//! `ledger::resolve_pending`, and appends what comes back. It never guesses: whatever the evidence
//! cannot place stays pending and is counted by reason.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{DefaultBranch, ReportError};
use crate::attribution::WorktreeMap;
use crate::git::{Git, GitError, Worktree};
use crate::ledger::{
    AppendOutcome, Clock, EvidenceMap, LedgerTarget, PendingEvidence, PendingStats, Salt, TallyDir,
    TimelineEvidence,
};
use crate::model::{
    AttributedRecord, CaptureEvent, Dispatch, Pending, Record, RequestId, Timestamp,
};
use crate::sources::{ReadStats, Source};

/// What [`resolve_at`] did.
#[derive(Debug)]
pub struct ResolveOutcome {
    /// The branch resolved for.
    pub branch: String,
    /// The repository's default branch, and how it was found.
    pub default_branch: DefaultBranch,
    /// True when `branch` is the default branch: nothing resolves to it, so nothing was read.
    pub on_default_branch: bool,
    /// What happened to every pending line, counted by reason.
    pub stats: PendingStats,
    /// Resolution lines appended. All zero on a dry run.
    pub appended: AppendOutcome,
    /// True when nothing was written.
    pub dry_run: bool,
    /// What the transcript read skipped.
    pub read_stats: ReadStats,
    /// As on `Report`.
    pub worktrees_unavailable: Option<GitError>,
    /// What reading `.tally/captures.jsonl` counted.
    pub capture_stats: crate::ledger::CaptureReadStats,
    /// What reading the ledger counted and skipped. A skipped line may have been a pending one,
    /// so the caller reports it (Governance II). All zeros when nothing was read.
    pub ledger_stats: crate::ledger::ReadStats,
}

/// Resolves the pending ledger lines that target `branch` (default: the current branch) by
/// appending resolution lines, from the live transcripts and the captures. Only that branch: there
/// is no resolving for every branch at once. With `dry_run` it counts and writes nothing.
///
/// On the default branch there is nothing to resolve to, so nothing is read
/// ([`ResolveOutcome::on_default_branch`]). A line that cannot be placed with certainty stays
/// pending and is counted by reason in [`ResolveOutcome::stats`]; none is guessed.
///
/// # Errors
///
/// [`ReportError`]: git cannot name the current branch, the source fails, discovery finds no
/// transcript directory, or the ledger cannot be read or written.
#[expect(clippy::too_many_arguments, reason = "mirrors `record_at`")]
pub fn resolve_at(
    git: &dyn Git,
    source: &dyn Source,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    target: &LedgerTarget<'_>,
    clock: &dyn Clock,
    dry_run: bool,
) -> Result<ResolveOutcome, ReportError> {
    let branch = super::branch_or_current(git, repo_dir, branch)?;
    // Decided with git alone: on the default branch nothing is resolved, so no transcript or
    // capture is read, and a repository without transcripts still gets the no-op outcome.
    let default_branch = super::resolve_default_branch(git, &git.main_checkout(repo_dir));
    let on_default_branch = branch == default_branch.name;
    let mut outcome = ResolveOutcome {
        branch,
        default_branch,
        on_default_branch,
        stats: PendingStats::default(),
        appended: AppendOutcome::default(),
        dry_run,
        read_stats: ReadStats::default(),
        worktrees_unavailable: None,
        capture_stats: crate::ledger::CaptureReadStats::default(),
        ledger_stats: crate::ledger::ReadStats::default(),
    };
    if !on_default_branch {
        let live = super::read_live(git, source, projects_root, repo_dir)?;
        outcome.read_stats = live.stats;
        outcome.worktrees_unavailable = live.unavailable;
        outcome.capture_stats = live.captures.stats().clone();
        let ran = Resolving {
            git,
            repo_dir,
            target,
            clock,
            branch: &outcome.branch,
            default_branch: &outcome.default_branch.name,
            dry_run,
        }
        .run(
            live.records,
            &live.dispatches,
            live.captures.events(),
            &live.main_checkout,
            &live.worktrees,
        )?;
        outcome.stats = ran.stats;
        outcome.appended = ran.appended;
        outcome.ledger_stats = ran.ledger_stats;
    }
    Ok(outcome)
}

/// What resolving a branch's pending lines needs besides the live data.
pub(super) struct Resolving<'a> {
    pub git: &'a dyn Git,
    pub repo_dir: &'a Path,
    pub target: &'a LedgerTarget<'a>,
    pub clock: &'a dyn Clock,
    pub branch: &'a str,
    pub default_branch: &'a str,
    pub dry_run: bool,
}

impl Resolving<'_> {
    /// Reads the ledger, gathers evidence, and appends the resolutions (unless a dry run).
    pub(super) fn run(
        &self,
        records: Vec<Record>,
        dispatches: &[Dispatch],
        captures: &[CaptureEvent],
        main: &Path,
        worktrees: &[Worktree],
    ) -> Result<Ran, ReportError> {
        let (root, path) = super::ledger_location(self.git, self.repo_dir, self.target)?;
        let ledger = match &root {
            Some(root) => crate::ledger::read_default(root)?,
            None => crate::ledger::read(&path)?,
        };
        let ledger_stats = ledger.stats().clone();
        if !ledger.entries().iter().any(|e| e.pending) {
            // Still carries the read stats: a skipped line may have been a pending one.
            return Ok(Ran {
                ledger_stats,
                ..Ran::default()
            });
        }
        let salts = matching_salts(&super::salt_homes(root.as_deref(), &path, main), &ledger);
        let evidence = gather(
            records,
            dispatches,
            captures,
            main,
            worktrees,
            self.default_branch,
            &salts,
        );
        let recorded_at = self.clock.now();
        let resolution =
            crate::ledger::resolve_pending(&ledger, self.branch, &evidence, &recorded_at);
        let appended = if self.dry_run || resolution.entries.is_empty() {
            AppendOutcome::default()
        } else {
            match &root {
                Some(root) => {
                    crate::ledger::append_entries_default(root, &resolution.entries, self.clock)?
                }
                None => crate::ledger::append_entries(&path, &resolution.entries, self.clock)?,
            }
        };
        Ok(Ran {
            stats: resolution.stats,
            appended,
            ledger_stats,
        })
    }
}

/// What [`Resolving::run`] did, and what its ledger read counted and skipped.
#[derive(Debug, Default)]
pub(super) struct Ran {
    pub stats: PendingStats,
    pub appended: AppendOutcome,
    pub ledger_stats: crate::ledger::ReadStats,
}

/// The salts in `homes` whose fingerprint is on a ledger line. A pending line can only be matched
/// under the salt it was redacted with, so a salt no line names is never used.
pub(super) fn matching_salts(homes: &[PathBuf], ledger: &crate::ledger::Ledger) -> Vec<Salt> {
    let wanted = &ledger.stats().redaction_fingerprints;
    let mut out: Vec<Salt> = Vec::new();
    if wanted.is_empty() {
        return out;
    }
    for home in homes {
        let loaded = TallyDir::open(home).and_then(|t| match t {
            Some(t) => Salt::load(&t),
            None => Ok(None),
        });
        if let Ok(Some(salt)) = loaded
            && wanted.contains(&salt.fingerprint())
            && !out.iter().any(|s| s.fingerprint() == salt.fingerprint())
        {
            out.push(salt);
        }
    }
    out
}

/// The evidence for every live request, keyed by the id a ledger line would hold: the raw id for
/// an unredacted line, and the hashed id under each matching salt for a redacted one.
fn gather(
    records: Vec<Record>,
    dispatches: &[Dispatch],
    captures: &[CaptureEvent],
    main: &Path,
    worktrees: &[Worktree],
    default_branch: &str,
    salts: &[Salt],
) -> EvidenceMap {
    let map = WorktreeMap::new(
        main,
        worktrees
            .iter()
            .map(|w| (w.path.as_path(), w.branch.as_str())),
    );
    let is_work = crate::attribution::default_work_branch(default_branch);
    let attributed = crate::attribution::attribute_timeline(records, dispatches, &map, &is_work);
    let mut out = EvidenceMap::new();
    for a in &attributed {
        let evidence = PendingEvidence {
            timeline: Some(TimelineEvidence::from_attributed(a)),
            capture: captured_work_branch(captures, a, &is_work),
        };
        let raw = &a.record.request_id;
        let keys = std::iter::once(raw.clone()).chain(salts.iter().map(|s| s.request_id(raw)));
        for key in keys {
            out.entry(key).or_insert_with(|| evidence.clone());
        }
    }
    out
}

/// The branch of the record's session's first capture *on a work branch* at or after the record's
/// `ts` (decision 0006): a capture still on the default branch, detached, or on an agent branch
/// places nothing and does not hide a later one. Sessions are compared raw: hashing is one-to-one,
/// so the hashed forms agree exactly when these do. Work-branch captures sharing that first instant
/// must agree; an unparseable capture gives no evidence.
fn captured_work_branch(
    captures: &[CaptureEvent],
    a: &AttributedRecord,
    is_work: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let usable = |branch: &str| {
        !branch.is_empty()
            && branch != "HEAD"
            && !branch.starts_with("worktree-agent-")
            && is_work(branch)
    };
    let ts = a.record.ts.as_ref()?.instant();
    let mut mine: Vec<(i128, &str)> = captures
        .iter()
        .filter(|c| c.session_id == a.record.session_id)
        .filter_map(|c| Some((c.ts.parse::<Timestamp>().ok()?.instant(), c.branch.as_str())))
        .filter(|(t, b)| *t >= ts && usable(b))
        .collect();
    mine.sort_by_key(|(t, _)| *t);
    let first = mine.first()?.0;
    let mut branches = mine
        .iter()
        .take_while(|(t, _)| *t == first)
        .map(|(_, b)| *b);
    let branch = branches.next()?;
    branches.all(|b| b == branch).then(|| branch.to_owned())
}

/// The ids under which a ledger line could hold the calls `ids` name: each raw id, and its hashed
/// form under every salt in `salts` (a redacted line is keyed by the hashed id).
pub(super) fn stored_forms<'a>(
    ids: impl Iterator<Item = &'a RequestId>,
    salts: &[Salt],
) -> HashSet<RequestId> {
    let mut out = HashSet::new();
    for id in ids {
        for salt in salts {
            out.insert(salt.request_id(id));
        }
        out.insert(id.clone());
    }
    out
}

/// The unresolved pending lines (as [`crate::ledger::Ledger::unresolved_pending`] counts them)
/// minus those whose request the report already counts in its totals (decision 0006): `counted`
/// holds the ids as a ledger line stores them. Deduplicated by request id.
pub(super) fn pending_outside(
    ledger: &crate::ledger::Ledger,
    counted: &HashSet<RequestId>,
) -> Pending {
    ledger.unresolved_pending_except(counted)
}

/// The request ids of `selected`, as the records carry them.
pub(super) fn request_ids(selected: &[crate::model::SelectedRecord]) -> Vec<RequestId> {
    selected
        .iter()
        .map(|s| s.attributed.record.request_id.clone())
        .collect()
}

/// A ledger report's pending usage: repository-wide, not this branch's, since pending lines wait
/// for a branch that is not this one yet. A request the totals already count is not also pending,
/// matched by its stored id (`selected`, hashed where a redacted line held it) and by the live
/// call's raw id (`live_raw`), so an unredacted pending line for a call matched in hashed form is
/// still excluded.
pub(super) fn ledger_report_pending(
    read: &crate::ledger::Ledger,
    homes: &[PathBuf],
    selected: &[crate::model::SelectedRecord],
    live_raw: &[RequestId],
) -> Pending {
    let salts = matching_salts(homes, read);
    let counted = stored_forms(
        selected
            .iter()
            .map(|s| &s.attributed.record.request_id)
            .chain(live_raw.iter()),
        &salts,
    );
    pending_outside(read, &counted)
}

/// The default ledger's unresolved pending lines that a live report's selection does not already
/// count, with what reading the ledger counted and skipped. Zero, and no stats, when there is no
/// checkout; zero when there is no ledger.
///
/// # Errors
/// The ledger exists but was refused or could not be read: pending usage is then unknown, which
/// the caller reports rather than reading it as zero (Governance II).
pub(super) fn live_pending(
    git: &dyn Git,
    repo_dir: &Path,
    main: &Path,
    selected: &[crate::model::SelectedRecord],
) -> Result<(Pending, Option<crate::ledger::ReadStats>), crate::ledger::LedgerError> {
    let Ok(root) = git.toplevel(repo_dir) else {
        return Ok((Pending::default(), None));
    };
    let ledger = crate::ledger::read_default(&root)?;
    let path = root.join(crate::ledger::LEDGER_PATH);
    let salts = matching_salts(&super::salt_homes(Some(&root), &path, main), &ledger);
    let counted = stored_forms(
        selected.iter().map(|s| &s.attributed.record.request_id),
        &salts,
    );
    Ok((
        pending_outside(&ledger, &counted),
        Some(ledger.stats().clone()),
    ))
}
