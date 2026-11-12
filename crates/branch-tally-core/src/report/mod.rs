//! The pipeline entry point: records in, [`Summary`] and [`Costs`] out.
//!
//! Chains attribution, summary and pricing in the order `main` does in `reference/token_report.py`
//! and hands the result to `render`. [`build`] is pure; [`run`] adds the reference's resolution
//! order (current branch, main checkout, worktree list) behind the [`Git`] and [`Source`] traits.

use std::path::{Path, PathBuf};

use crate::git::{Git, GitError, Worktree};
use crate::ledger::LedgerTarget;
use crate::model::{Costs, DefaultBranchSource, Dispatch, Mode, Record, Summary, WorkBranchRule};
use crate::pricing::PriceTable;
use crate::redact::RedactOptions;
use crate::sources::{ReadStats, RepoIdentity, Source, SourceError};

/// A finished report: what the renderers take, plus what the read skipped.
#[derive(Debug)]
pub struct Report {
    /// The branch that was reported on.
    pub branch: String,
    /// Picked records, aggregated.
    pub summary: Summary,
    /// Per-model cost and the total.
    pub costs: Costs,
    /// What the source read and skipped. Data only; the renderers do not print it.
    pub read_stats: ReadStats,
    /// Set by [`run`] when the worktree list could not be read. The reference treats that as
    /// "no worktrees"; the report proceeds the same way but says so here.
    pub worktrees_unavailable: Option<GitError>,
    /// Set by [`run_from_ledger`]: where the numbers came from. `None` for a live report.
    pub ledger: Option<LedgerReport>,
    /// What reading `.tally/captures.jsonl` counted. All zeros when there is no captures file.
    /// Bad lines are counted here and never dropped silently.
    pub capture_stats: crate::ledger::CaptureReadStats,
}

/// Whether the repository-wide counts (`unattributed`, and the full ambiguous set) were seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullSetCounts {
    /// Ledger only: the ledger holds selected records, never the full attributed set, so the
    /// unattributed counts are **unknown**, not zero, and the ambiguous count is a floor built
    /// from the ledger's own ambiguous entries.
    Unavailable,
    /// Unioned with live transcripts: computed over the live full attributed set plus the
    /// ledger's entries.
    FromLive,
}

/// What a ledger-backed report was built from. Counts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerReport {
    /// What reading the ledger skipped, deduplicated or resolved.
    pub ledger_stats: crate::ledger::ReadStats,
    /// Records only the ledger held, for this branch.
    pub ledger_only: u64,
    /// Records only the live transcripts held (zero when no live side was read).
    pub live_only: u64,
    /// Request ids both held; the ledger's copy was used.
    pub in_both: u64,
    /// The distinct modes this branch's entries were recorded under. More than one means the
    /// numbers mix modes; the caller says so.
    pub modes: Vec<Mode>,
    /// Whether live transcripts were read and unioned.
    pub live_included: bool,
    /// Whether the repository-wide counts were seen.
    pub full_set: FullSetCounts,
    /// Live read stats when `live_included`.
    pub live_read_stats: Option<ReadStats>,
}

/// What [`record`] did.
#[derive(Debug)]
pub struct Recorded {
    /// The branch recorded.
    pub branch: String,
    /// Entries appended and skipped as already held.
    pub outcome: crate::ledger::AppendOutcome,
    /// Records the pipeline selected for the branch (`appended + skipped`).
    pub selected: u64,
    /// What the transcript read skipped.
    pub read_stats: ReadStats,
    /// As on [`Report`].
    pub worktrees_unavailable: Option<GitError>,
    /// What reading `.tally/captures.jsonl` counted, as on [`Report`].
    pub capture_stats: crate::ledger::CaptureReadStats,
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
    /// Discovery found no transcript directory for the repository (#40). The message is the
    /// reference's sentence, naming every slug that was tried.
    #[error("No transcript directory for {} ({})", .repo.display(), expected_slugs(.slugs))]
    NoTranscriptDirectory {
        /// The main checkout transcripts were looked for.
        repo: PathBuf,
        /// Every slug that was tried, lowercased.
        slugs: Vec<String>,
    },
    /// The current branch could not be determined.
    #[error(transparent)]
    Git(#[from] GitError),
    /// Transcripts could not be discovered or read.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// The ledger could not be read or written.
    #[error(transparent)]
    Ledger(#[from] crate::ledger::LedgerError),
    /// A ledger-only report was asked for and there is no ledger file.
    #[error("no ledger at {}", .path.display())]
    LedgerMissing {
        /// Where the ledger was expected.
        path: PathBuf,
    },
}

/// The error for a repository with no transcript directory: every candidate slug, lowercased
/// (slugs are compared case-insensitively) and deduplicated.
fn no_transcript_directory(repo: PathBuf) -> ReportError {
    let mut slugs: Vec<String> = Vec::new();
    for slug in crate::paths::slug_candidates(&repo) {
        let slug = slug.to_lowercase();
        if !slugs.contains(&slug) {
            slugs.push(slug);
        }
    }
    ReportError::NoTranscriptDirectory { repo, slugs }
}

/// `expected slug a` for one slug (the reference's wording), `expected one of the slugs a, b`
/// for several.
fn expected_slugs(slugs: &[String]) -> String {
    match slugs {
        [one] => format!("expected slug {one}"),
        many => format!("expected one of the slugs {}", many.join(", ")),
    }
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
    /// Redact the summary after it is built and before it is priced (#9). `None`, the default,
    /// leaves the report untouched.
    pub redaction: Option<RedactOptions>,
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
            redaction: None,
        }
    }

    /// Redact identifying detail from the summary. Costs are unchanged.
    #[must_use]
    pub fn with_redaction(mut self, redaction: RedactOptions) -> Self {
        self.redaction = Some(redaction);
        self
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

/// The selection step both [`build`] and the ledger entries share: what attribution kept for a
/// branch, and the counts that come from the full attributed set.
struct Picked {
    /// Every attributed record, selected or not.
    attributed: Vec<crate::model::AttributedRecord>,
    selected: Vec<crate::model::SelectedRecord>,
    unattributed: (u64, u64),
    ambiguous: crate::model::HeuristicCount,
    work_rule: Option<WorkBranchRule>,
}

/// Attributes `records` and selects those kept for `branch`. Pure.
fn select_for(
    records: Vec<Record>,
    dispatches: &[Dispatch],
    captures: &[crate::model::CaptureEvent],
    main_checkout: &Path,
    worktrees: &[Worktree],
    branch: &str,
    options: &Options<'_>,
) -> Picked {
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
        crate::attribution::attribute_with_captures(records, dispatches, captures, &map, strict)
    };
    let unattributed = crate::model::SubagentSummary::count_unattributed(&attributed);
    let all = attributed.clone();
    let selected = if mode == Mode::Timeline {
        crate::attribution::select_timeline(attributed, branch)
    } else {
        crate::attribution::select(attributed, branch, strict)
    };
    let ambiguous = crate::summary::count_ambiguous(&all, &selected);
    Picked {
        attributed: all,
        selected,
        unattributed,
        ambiguous,
        work_rule,
    }
}

/// Summarizes and prices a selection.
fn finish(
    branch: &str,
    options: &Options<'_>,
    sel: Picked,
    read_stats: ReadStats,
    table: &PriceTable,
) -> Report {
    let mut summary = crate::summary::summarize(
        branch,
        options.mode,
        &sel.selected,
        sel.unattributed,
        sel.ambiguous,
    );
    summary.work_branch = sel.work_rule;
    if let Some(redaction) = options.redaction {
        summary = crate::redact::redact(summary, redaction);
    }
    let costs = crate::pricing::price(&summary, table);
    Report {
        branch: branch.to_owned(),
        summary,
        costs,
        read_stats,
        worktrees_unavailable: None,
        ledger: None,
        capture_stats: crate::ledger::CaptureReadStats::default(),
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
    build_with_captures(
        records,
        dispatches,
        &[],
        read_stats,
        main_checkout,
        worktrees,
        branch,
        options,
        table,
    )
}

/// [`build`] plus hook captures (#13), which [`crate::attribution::attribute_with_captures`]
/// treats as authoritative over `worktrees` for records whose stamp is missing or
/// `worktree-agent-<id>`. Pure: the caller read `captures` from `.tally/captures.jsonl` and keeps
/// their read stats. The summary's captured heuristic count says when they were used.
///
/// # Errors
///
/// [`ReportError::NoRecords`] when nothing is selected.
#[expect(
    clippy::too_many_arguments,
    reason = "a pure pipeline entry point; each input is a distinct stage input"
)]
pub fn build_with_captures(
    records: Vec<Record>,
    dispatches: &[Dispatch],
    captures: &[crate::model::CaptureEvent],
    read_stats: ReadStats,
    main_checkout: &Path,
    worktrees: &[Worktree],
    branch: &str,
    options: &Options<'_>,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    let sel = select_for(
        records,
        dispatches,
        captures,
        main_checkout,
        worktrees,
        branch,
        options,
    );
    if sel.selected.is_empty() {
        return Err(ReportError::NoRecords {
            branch: branch.to_owned(),
            read_stats,
        });
    }
    Ok(finish(branch, options, sel, read_stats, table))
}

/// Everything read from live transcripts, before attribution.
struct Live {
    records: Vec<Record>,
    dispatches: Vec<Dispatch>,
    /// Hook captures from the main checkout's `.tally/captures.jsonl`; empty when there is none.
    captures: crate::ledger::Captures,
    stats: ReadStats,
    main_checkout: PathBuf,
    worktrees: Vec<Worktree>,
    unavailable: Option<GitError>,
}

/// Discovers and reads the transcripts for the main checkout of `repo_dir`.
fn read_live(
    git: &dyn Git,
    source: &dyn Source,
    projects_root: &Path,
    repo_dir: &Path,
) -> Result<Live, ReportError> {
    let main_checkout = git.main_checkout(repo_dir);
    let roots = source.discover(projects_root, &RepoIdentity::new(&main_checkout))?;
    if roots.is_empty() {
        return Err(no_transcript_directory(main_checkout));
    }
    let mut records = Vec::new();
    let stats = source.read(&roots, &mut |r| records.push(r))?;
    let dispatches = source.dispatches(&roots)?;
    let (worktrees, unavailable) = match git.worktrees(&main_checkout) {
        Ok(w) => (w, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    // A missing file is no captures; an unreadable one is an error, as an unreadable ledger is.
    let captures = crate::ledger::read_captures(&main_checkout)?;
    Ok(Live {
        records,
        dispatches,
        captures,
        stats,
        main_checkout,
        worktrees,
        unavailable,
    })
}

/// Fills in the default branch for timeline mode with the default rule; asks git only then.
fn resolve_options<'o>(
    git: &dyn Git,
    main_checkout: &Path,
    options: &'o Options<'o>,
    storage: &'o mut Option<Options<'o>>,
) -> &'o Options<'o> {
    if options.mode == Mode::Timeline
        && matches!(options.work_branch, WorkBranch::Default)
        && options.default_branch.is_none()
    {
        let resolved = Options {
            default_branch: Some(resolve_default_branch(git, main_checkout)),
            ..options.clone()
        };
        return storage.insert(resolved);
    }
    options
}

fn branch_or_current(
    git: &dyn Git,
    repo_dir: &Path,
    branch: Option<&str>,
) -> Result<String, GitError> {
    match branch {
        Some(b) => Ok(b.to_owned()),
        None => git.current_branch(repo_dir),
    }
}

/// The reference's `main()` resolution: `branch` defaults to the current branch of `repo_dir`,
/// the main checkout comes from git's common dir, and the worktree list from git. Transcripts
/// are discovered under `projects_root` for the main checkout.
///
/// # Errors
///
/// [`ReportError`]: git cannot name the current branch, the source fails, discovery finds no
/// transcript directory for the repository, or nothing is selected.
pub fn run(
    git: &dyn Git,
    source: &dyn Source,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    options: &Options<'_>,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    let branch = branch_or_current(git, repo_dir, branch)?;
    let live = read_live(git, source, projects_root, repo_dir)?;
    let mut storage = None;
    let options = resolve_options(git, &live.main_checkout, options, &mut storage);
    let mut report = build_with_captures(
        live.records,
        &live.dispatches,
        live.captures.events(),
        live.stats,
        &live.main_checkout,
        &live.worktrees,
        &branch,
        options,
        table,
    )?;
    report.worktrees_unavailable = live.unavailable;
    report.capture_stats = live.captures.stats().clone();
    Ok(report)
}

/// Selects `branch`'s records exactly as [`run`] would, then appends the ones the ledger at
/// `ledger_path` does not already hold, through `redact`, stamped by `clock`. Idempotent.
///
/// A branch with nothing selected is `Ok` with `selected == 0` (nothing to record is not a
/// failure); the caller says so.
///
/// # Errors
///
/// [`ReportError`]: as [`run`], plus [`ReportError::Ledger`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `run` plus the ledger seams"
)]
pub fn record(
    git: &dyn Git,
    source: &dyn Source,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    options: &Options<'_>,
    ledger_path: &Path,
    redact: &dyn crate::ledger::Redact,
    clock: &dyn crate::ledger::Clock,
) -> Result<Recorded, ReportError> {
    record_at(
        git,
        source,
        projects_root,
        repo_dir,
        branch,
        options,
        &LedgerTarget::Explicit(ledger_path),
        redact,
        clock,
    )
}

/// [`record`] with the ledger chosen by a [`LedgerTarget`]: [`LedgerTarget::Default`] writes
/// `.tally/ledger.jsonl` at `git.toplevel(repo_dir)` through the guarded accessor (#94), so a
/// linked worktree writes its own ledger, not the main checkout's. It writes only when that
/// checkout's `.tally/` exists, and says so in `outcome.tally_absent` when it does not.
/// [`LedgerTarget::Explicit`] is the plain write to a user-supplied path.
///
/// # Errors
///
/// As [`record`]; a refused symlink is [`ReportError::Ledger`] holding
/// [`LedgerError::Refused`](crate::ledger::LedgerError::Refused).
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `run` plus the ledger seams"
)]
pub fn record_at(
    git: &dyn Git,
    source: &dyn Source,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    options: &Options<'_>,
    target: &LedgerTarget<'_>,
    redact: &dyn crate::ledger::Redact,
    clock: &dyn crate::ledger::Clock,
) -> Result<Recorded, ReportError> {
    let branch = branch_or_current(git, repo_dir, branch)?;
    let live = read_live(git, source, projects_root, repo_dir)?;
    let mut storage = None;
    let options = resolve_options(git, &live.main_checkout, options, &mut storage);
    let sel = select_for(
        live.records,
        &live.dispatches,
        live.captures.events(),
        &live.main_checkout,
        &live.worktrees,
        &branch,
        options,
    );
    let outcome = match target {
        // The ledger is per checkout: a linked worktree records on its own branch (decision
        // 0002), so the root is the git top level, as `--ledger`'s default is. Outside a
        // repository that is an error, never a fallback.
        LedgerTarget::Default => crate::ledger::append_default(
            &git.toplevel(repo_dir)?,
            &sel.selected,
            &branch,
            options.mode,
            Some(&live.main_checkout),
            redact,
            clock,
        )?,
        LedgerTarget::Explicit(path) => crate::ledger::append(
            path,
            &sel.selected,
            &branch,
            options.mode,
            Some(&live.main_checkout),
            redact,
            clock,
        )?,
    };
    Ok(Recorded {
        branch,
        outcome,
        selected: sel.selected.len() as u64,
        read_stats: live.stats,
        worktrees_unavailable: live.unavailable,
        capture_stats: live.captures.stats().clone(),
    })
}

/// Reports from the ledger at `ledger_path`, optionally unioned with the live transcripts of
/// `source` (the ledger's copy wins on overlap).
///
/// The ledger holds only selected records, so a ledger-only report cannot know the
/// repository-wide `unattributed` counts: [`LedgerReport::full_set`] is
/// [`FullSetCounts::Unavailable`], the summary carries zeros there, and the caller must say they
/// are unknown. Its ambiguous count is a floor from the ledger's own entries. With a live side,
/// both come from the live full attributed set plus the ledger's entries.
///
/// `options.mode` labels the summary; the ledger does not store the mode it was recorded under.
///
/// # Errors
///
/// [`ReportError::LedgerMissing`] when there is no ledger file and no live side;
/// [`ReportError::NoRecords`] when nothing is selected; otherwise as [`run`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `run` plus the ledger path"
)]
pub fn run_from_ledger(
    git: &dyn Git,
    source: Option<&dyn Source>,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    options: &Options<'_>,
    ledger_path: &Path,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    run_from_ledger_at(
        git,
        source,
        projects_root,
        repo_dir,
        branch,
        options,
        &LedgerTarget::Explicit(ledger_path),
        table,
    )
}

/// [`run_from_ledger`] with the ledger chosen by a [`LedgerTarget`]: [`LedgerTarget::Default`]
/// reads `.tally/ledger.jsonl` at `git.toplevel(repo_dir)` through the guarded accessor (#94), so
/// a linked worktree reads its own ledger, and refuses a symlink. [`LedgerTarget::Explicit`] is
/// the plain read of a user-supplied path.
///
/// # Errors
///
/// As [`run_from_ledger`]; a refused symlink is [`ReportError::Ledger`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `run` plus the ledger path"
)]
pub fn run_from_ledger_at(
    git: &dyn Git,
    source: Option<&dyn Source>,
    projects_root: &Path,
    repo_dir: &Path,
    branch: Option<&str>,
    options: &Options<'_>,
    target: &LedgerTarget<'_>,
    table: &PriceTable,
) -> Result<Report, ReportError> {
    let branch = branch_or_current(git, repo_dir, branch)?;
    let main_checkout = git.main_checkout(repo_dir);
    // Per-checkout ledger: the git top level for the default (see `record_at`).
    let ledger_root = match target {
        LedgerTarget::Default => Some(git.toplevel(repo_dir)?),
        LedgerTarget::Explicit(_) => None,
    };
    let ledger_path = match (target, &ledger_root) {
        (LedgerTarget::Explicit(path), _) => path.to_path_buf(),
        (LedgerTarget::Default, Some(root)) => root.join(crate::ledger::LEDGER_PATH),
        (LedgerTarget::Default, None) => PathBuf::new(),
    };
    // The default is checked with `symlink_metadata`, so a planted link counts as present and the
    // guarded read below refuses it, rather than this check calling the ledger missing.
    let present = match target {
        LedgerTarget::Default => std::fs::symlink_metadata(&ledger_path).is_ok(),
        LedgerTarget::Explicit(_) => ledger_path.exists(),
    };
    if source.is_none() && !present {
        return Err(ReportError::LedgerMissing { path: ledger_path });
    }
    let read = match &ledger_root {
        Some(root) => crate::ledger::read_default(root)?,
        None => crate::ledger::read(&ledger_path)?,
    };
    // Exactly the entries `record` wrote for this branch; the ledger keys on (request, branch).
    let ledger = read.for_branch(&branch);
    let ledger_records = ledger.to_selected(Some(&main_checkout));
    let mut info = LedgerReport {
        ledger_stats: read.stats().clone(),
        ledger_only: ledger_records.len() as u64,
        live_only: 0,
        in_both: 0,
        modes: ledger.modes(),
        live_included: source.is_some(),
        full_set: FullSetCounts::Unavailable,
        live_read_stats: None,
    };
    let mut unavailable = None;
    let mut capture_stats = crate::ledger::CaptureReadStats::default();
    let mut storage = None;
    let (sel, read_stats) = if let Some(source) = source {
        let live = read_live(git, source, projects_root, repo_dir)?;
        let live_options = resolve_options(git, &live.main_checkout, options, &mut storage);
        let live_sel = select_for(
            live.records,
            &live.dispatches,
            live.captures.events(),
            &live.main_checkout,
            &live.worktrees,
            &branch,
            live_options,
        );
        unavailable = live.unavailable;
        capture_stats = live.captures.stats().clone();
        // The ledger's copy wins on overlap.
        let union = crate::ledger::union(&ledger, &live_sel.selected, Some(&main_checkout));
        let records = union.records;
        info.ledger_only = union.ledger_only;
        info.live_only = union.live_only;
        info.in_both = union.in_both;
        // Repository-wide counts: the live full set, plus ledger entries live no longer holds.
        let mut all = live_sel.attributed;
        let live_ids: std::collections::HashSet<_> =
            all.iter().map(|a| a.record.request_id.clone()).collect();
        all.extend(
            records
                .iter()
                .filter(|r| !live_ids.contains(&r.attributed.record.request_id))
                .map(|r| r.attributed.clone()),
        );
        info.full_set = FullSetCounts::FromLive;
        info.live_read_stats = Some(live.stats);
        let sel = Picked {
            unattributed: crate::model::SubagentSummary::count_unattributed(&all),
            ambiguous: crate::summary::count_ambiguous(&all, &records),
            attributed: all,
            selected: records,
            work_rule: live_sel.work_rule,
        };
        (sel, live.stats)
    } else {
        let attributed: Vec<_> = ledger_records
            .iter()
            .map(|r| r.attributed.clone())
            .collect();
        let ambiguous = crate::summary::count_ambiguous(&attributed, &ledger_records);
        let sel = Picked {
            attributed,
            selected: ledger_records,
            unattributed: (0, 0),
            ambiguous,
            work_rule: None,
        };
        (sel, ReadStats::default())
    };
    if sel.selected.is_empty() {
        return Err(ReportError::NoRecords { branch, read_stats });
    }
    let mut report = finish(&branch, options, sel, read_stats, table);
    report.worktrees_unavailable = unavailable;
    report.capture_stats = capture_stats;
    report.ledger = Some(info);
    Ok(report)
}
