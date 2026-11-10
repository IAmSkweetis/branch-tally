//! The pipeline entry point: records in, [`Summary`] and [`Costs`] out.
//!
//! Chains attribution, summary and pricing in the order `main` does in `reference/token_report.py`
//! and hands the result to `render`. [`build`] is pure; [`run`] adds the reference's resolution
//! order (current branch, main checkout, worktree list) behind the [`Git`] and [`Source`] traits.

use std::path::Path;

use crate::git::{Git, GitError, Worktree};
use crate::model::{Costs, Mode, Record, Summary};
use crate::pricing::PriceTable;
use crate::sources::{ReadStats, RepoIdentity, Source, SourceError};

/// A finished report: what the renderers take, plus what the read skipped.
#[derive(Debug)]
pub struct Report {
    /// The branch that was reported on.
    pub branch: String,
    /// Selected records, aggregated.
    pub summary: Summary,
    /// Per-model cost and the total.
    pub costs: Costs,
    /// What the source read and skipped. Data only; the renderers do not print it.
    pub read_stats: ReadStats,
    /// Set by [`run`] when the worktree list could not be read. The reference treats that as
    /// "no worktrees"; the report proceeds the same way but says so here.
    pub worktrees_unavailable: Option<GitError>,
}

/// Why no report was produced.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// No record was selected for the branch (the reference exits "No transcript records found").
    #[error("no transcript records found for branch {branch:?}")]
    NoRecords {
        /// The branch asked for.
        branch: String,
        /// What the read skipped, so a caller can explain an empty result.
        read_stats: ReadStats,
    },
    /// The current branch could not be determined.
    #[error(transparent)]
    Git(#[from] GitError),
    /// Transcripts could not be discovered or read.
    #[error(transparent)]
    Source(#[from] SourceError),
}

/// Attributes, selects, summarizes and prices `records` for `branch`. Pure.
///
/// `worktrees` is git's branch-holding worktree list, which includes the main checkout itself
/// (kept as the reference does; #37).
///
/// `table` prices the summary; the caller resolves it (override, cache or bundled).
///
/// # Errors
///
/// [`ReportError::NoRecords`] when nothing is selected.
pub fn build(
    records: Vec<Record>,
    read_stats: ReadStats,
    main_checkout: &Path,
    worktrees: &[Worktree],
    branch: &str,
    strict: bool,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    let map = crate::attribution::WorktreeMap::new(
        main_checkout,
        worktrees
            .iter()
            .map(|w| (w.path.as_path(), w.branch.as_str())),
    );
    let attributed = crate::attribution::attribute(records, &[], &map, strict);
    let selected = crate::attribution::select(attributed, branch, strict);
    if selected.is_empty() {
        return Err(ReportError::NoRecords {
            branch: branch.to_owned(),
            read_stats,
        });
    }
    let mode = if strict { Mode::Strict } else { Mode::Claim };
    let summary = crate::summary::summarize(branch, mode, &selected);
    let costs = crate::pricing::price(&summary, table);
    Ok(Report {
        branch: branch.to_owned(),
        summary,
        costs,
        read_stats,
        worktrees_unavailable: None,
    })
}

/// The reference's `main()` resolution: `branch` defaults to the current branch of `repo_dir`,
/// the main checkout comes from git's common dir, and the worktree list from git. Transcripts
/// are discovered under `projects_root` for the main checkout.
///
/// # Errors
///
/// [`ReportError`]: git cannot name the current branch, the source fails, or nothing is selected.
pub fn run(
    git: &dyn Git,
    source: &dyn Source,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    strict: bool,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    let branch = match branch {
        Some(b) => b.to_owned(),
        None => git.current_branch(repo_dir)?,
    };
    let main_checkout = git.main_checkout(repo_dir);
    let roots = source.discover(projects_root, &RepoIdentity::new(&main_checkout))?;
    let mut records = Vec::new();
    let stats = source.read(&roots, &mut |r| records.push(r))?;
    let (worktrees, unavailable) = match git.worktrees(&main_checkout) {
        Ok(w) => (w, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let mut report = build(
        records,
        stats,
        &main_checkout,
        &worktrees,
        &branch,
        strict,
        table,
    )?;
    report.worktrees_unavailable = unavailable;
    Ok(report)
}
