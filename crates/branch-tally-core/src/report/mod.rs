//! The pipeline entry point: records in, [`Summary`] and [`Costs`] out.
//!
//! Chains attribution, summary and pricing in the order `main` does in `reference/token_report.py`
//! and hands the result to `render`. [`build`] is pure; [`run`] adds the reference's resolution
//! order (current branch, main checkout, worktree list) behind the [`Git`] and [`Source`] traits.

use std::path::Path;

use crate::git::{Git, GitError, Worktree};
use crate::model::{Costs, DefaultBranchSource, Dispatch, Mode, Record, Summary, WorkBranchRule};
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

/// The default branch timeline mode excludes from the work branches, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultBranch {
    /// The branch name.
    pub name: String,
    /// How it was found. Anything but `OriginHead` is a fallback the report states.
    pub source: DefaultBranchSource,
}

/// Which stamped branches count as work branches in timeline mode.
#[derive(Clone, Copy)]
pub enum WorkBranch<'a> {
    /// Every branch except the repository's default branch and `worktree-agent-*`.
    Default,
    /// The caller's own rule, from `--work-branch`. The CLI compiles the pattern (core has no
    /// regex engine); core only calls `matches` and shows `pattern` in the report.
    Custom {
        /// The pattern text as the user gave it. Shown in the report.
        pattern: &'a str,
        /// `true` for a branch name that is a work branch.
        matches: &'a dyn Fn(&str) -> bool,
    },
}

impl std::fmt::Debug for WorkBranch<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("Default"),
            Self::Custom { pattern, .. } => {
                f.debug_struct("Custom").field("pattern", pattern).finish()
            }
        }
    }
}

/// How to select and attribute: the mode, plus what timeline mode needs.
///
/// Build with [`Options::new`] and chain the setters; `Options::default()` is claim mode, the
/// product default.
#[derive(Debug, Clone)]
pub struct Options<'a> {
    /// Strict, claim (default) or timeline.
    pub mode: Mode,
    /// Timeline only: the work-branch rule. Ignored by strict and claim.
    pub work_branch: WorkBranch<'a>,
    /// Timeline with [`WorkBranch::Default`] only: the resolved default branch. [`run`] resolves
    /// it from git when this is `None`; [`build`] (pure, no git) assumes `main` and says so.
    pub default_branch: Option<DefaultBranch>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self::new(Mode::Claim)
    }
}

impl<'a> Options<'a> {
    /// Options for `mode` with the default work-branch rule.
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            work_branch: WorkBranch::Default,
            default_branch: None,
        }
    }

    /// Use a custom work-branch rule (timeline mode).
    #[must_use]
    pub fn with_work_branch(mut self, work_branch: WorkBranch<'a>) -> Self {
        self.work_branch = work_branch;
        self
    }

    /// Supply an already-resolved default branch (timeline mode, default rule).
    #[must_use]
    pub fn with_default_branch(mut self, default_branch: DefaultBranch) -> Self {
        self.default_branch = Some(default_branch);
        self
    }
}

/// A work-branch predicate, boxed because the default and custom rules are different types.
type WorkPredicate<'a> = Box<dyn Fn(&str) -> bool + 'a>;

/// The rule to record on the summary and the predicate timeline attribution uses.
fn work_branch_rule<'a>(options: &'a Options<'_>) -> (WorkBranchRule, WorkPredicate<'a>) {
    match options.work_branch {
        WorkBranch::Custom { pattern, matches } => (
            WorkBranchRule::Custom {
                pattern: pattern.to_owned(),
            },
            Box::new(matches),
        ),
        WorkBranch::Default => {
            // Pure: with nothing resolved, assume `main`, and the summary says it was assumed.
            let default = options.default_branch.clone().unwrap_or(DefaultBranch {
                name: "main".to_owned(),
                source: DefaultBranchSource::AssumedMain,
            });
            let is_work = crate::attribution::default_work_branch(&default.name);
            (
                WorkBranchRule::ExcludesDefault {
                    branch: default.name,
                    source: default.source,
                },
                Box::new(is_work),
            )
        }
    }
}

/// Finds the default branch for timeline mode: `origin/HEAD`, else `init.defaultBranch`, else
/// `main` assumed. Never fails; the source says which was used.
fn resolve_default_branch(git: &dyn Git, main_checkout: &Path) -> DefaultBranch {
    if let Ok(name) = git.default_branch(main_checkout) {
        return DefaultBranch {
            name,
            source: DefaultBranchSource::OriginHead,
        };
    }
    if let Ok(name) = git.init_default_branch(main_checkout) {
        return DefaultBranch {
            name,
            source: DefaultBranchSource::InitDefaultBranch,
        };
    }
    DefaultBranch {
        name: "main".to_owned(),
        source: DefaultBranchSource::AssumedMain,
    }
}

/// Attributes, selects, summarizes and prices `records` for `branch`. Pure.
///
/// `worktrees` is git's branch-holding worktree list, which includes the main checkout itself.
/// Attribution never recovers a branch from the main checkout's entry (#37).
///
/// `dispatches` link dispatching sessions to their subagents. They roll subagent records up to
/// the dispatcher's branch (#6) and are ignored in strict mode.
///
/// `options` picks the mode (#7). In timeline mode with the default rule and no resolved default
/// branch, `main` is assumed, and the summary says so.
///
/// `table` prices the summary; the caller resolves it (override, cache or bundled).
///
/// # Errors
///
/// [`ReportError::NoRecords`] when nothing is selected.
#[expect(
    clippy::too_many_arguments,
    reason = "a pure pipeline entry point; each input is a distinct stage input"
)]
pub fn build(
    records: Vec<Record>,
    dispatches: &[Dispatch],
    read_stats: ReadStats,
    main_checkout: &Path,
    worktrees: &[Worktree],
    branch: &str,
    options: &Options<'_>,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    let map = crate::attribution::WorktreeMap::new(
        main_checkout,
        worktrees
            .iter()
            .map(|w| (w.path.as_path(), w.branch.as_str())),
    );
    let mode = options.mode;
    let strict = mode == Mode::Strict;
    let mut work_rule = None;
    let attributed = if mode == Mode::Timeline {
        let (rule, is_work) = work_branch_rule(options);
        work_rule = Some(rule);
        crate::attribution::attribute_timeline(records, dispatches, &map, &*is_work)
    } else {
        crate::attribution::attribute(records, dispatches, &map, strict)
    };
    let unattributed = crate::model::SubagentSummary::count_unattributed(&attributed);
    let attributed_all = attributed.clone();
    let selected = if mode == Mode::Timeline {
        crate::attribution::select_timeline(attributed, branch)
    } else {
        crate::attribution::select(attributed, branch, strict)
    };
    let ambiguous = crate::summary::count_ambiguous(&attributed_all, &selected);
    if selected.is_empty() {
        return Err(ReportError::NoRecords {
            branch: branch.to_owned(),
            read_stats,
        });
    }
    let mut summary = crate::summary::summarize(branch, mode, &selected, unattributed, ambiguous);
    summary.work_branch = work_rule.take();
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
    options: &Options<'_>,
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
    let dispatches = source.dispatches(&roots)?;
    let (worktrees, unavailable) = match git.worktrees(&main_checkout) {
        Ok(w) => (w, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    // Only timeline mode with the default rule needs the default branch; ask git only then.
    let resolved;
    let options = if options.mode == Mode::Timeline
        && matches!(options.work_branch, WorkBranch::Default)
        && options.default_branch.is_none()
    {
        resolved = Options {
            default_branch: Some(resolve_default_branch(git, &main_checkout)),
            ..options.clone()
        };
        &resolved
    } else {
        options
    };
    let mut report = build(
        records,
        &dispatches,
        stats,
        &main_checkout,
        &worktrees,
        &branch,
        options,
        table,
    )?;
    report.worktrees_unavailable = unavailable;
    Ok(report)
}
