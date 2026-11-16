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

mod pending;

pub use pending::{ResolveOutcome, resolve_at};

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
    /// Set by [`run`] when the default ledger exists but could not be read (refused or
    /// unreadable). Pending usage is then unknown, not zero: the report proceeds and says so here.
    pub pending_unavailable: Option<crate::ledger::LedgerError>,
    /// Set by [`run`] when it read the default ledger for pending usage: what that read counted and
    /// skipped. A skipped line may have been a pending one, so the caller reports it.
    pub pending_ledger_stats: Option<crate::ledger::ReadStats>,
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
    /// Whether live records could be matched against the ledger's redacted lines.
    pub live_redaction: LiveRedactionMatch,
    /// Lines collapsed because one call was held under both spellings of an agent branch (raw and
    /// hashed) with the same request id: the kept line is the latest `recorded_at`, then the
    /// smallest serialized line. Counted here so the caller can say so; never hidden.
    pub collapsed_lines: u64,
}

/// Whether live transcripts could be matched against a redacted ledger (decision 0005).
///
/// Anything other than [`NotNeeded`](Self::NotNeeded) or [`Matched`](Self::Matched) means some
/// live calls could not be recognised in their redacted lines, so they may be **counted twice**
/// (the total may be an overcount, not a floor) and the repository-wide counts may be inflated.
/// The report must say so (Governance II).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveRedactionMatch {
    /// No live side, or the ledger holds no redacted lines.
    NotNeeded,
    /// The local salt matches every redacted line's fingerprint.
    Matched,
    /// The ledger has redacted lines and there is no `.tally/salt` here.
    NoSalt,
    /// `.tally/salt` exists but could not be read as a salt.
    SaltUnreadable,
    /// The local salt's fingerprint is on none of the redacted lines.
    FingerprintMismatch,
    /// The salt matches some redacted lines; others carry a different fingerprint and cannot be
    /// matched.
    PartiallyMatched,
}

/// How [`record`] and [`record_at`] redact the lines they append (decision 0005).
///
/// `Keyed` resolves the salt itself, after the target `.tally/` is opened and from that same
/// directory, and creates one only when at least one line will actually be appended. The caller
/// never holds a salt, so no line can be hashed under one that is not persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordRedaction {
    /// Write ids as the harness recorded them.
    Off,
    /// Hash ids (and, with `names`, agent and skill names) under the target's salt.
    Keyed {
        /// Also hash agent and skill names.
        names: bool,
    },
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
    /// Lines written pending (decision 0006): `outcome.appended` when `branch` is the repository's
    /// default branch, else zero. Their usage is uncertain until a branch is cut and resolved.
    pub pending_written: u64,
    /// Set when `branch` is not the default branch: what resolving the pending lines that target
    /// it did, counted by reason. `None` on the default branch, where nothing resolves.
    pub resolution: Option<crate::ledger::PendingStats>,
    /// Resolution lines appended to the ledger by that resolution.
    pub resolutions_appended: u64,
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
    /// Keyed redaction of an explicit `--ledger` needs the main checkout's `.tally/` to hold the
    /// salt, and there is none.
    #[error("ledger redaction needs a .tally/ directory in the main checkout to hold its salt")]
    NoSaltHome,
    /// A ledger-only report of a `worktree-agent-<id>` branch whose ledger holds redacted lines,
    /// with no salt matching them: the hashed spelling can't be computed, so those lines can't be
    /// selected. Refused rather than reported short (Governance II).
    #[error(
        "can't select {branch} from a redacted ledger without its salt: agent branches are stored \
         hashed. Restore the matching .tally/salt"
    )]
    AgentBranchNeedsSalt {
        /// The requested agent branch.
        branch: String,
    },
    /// A ledger-only report of a `worktree-agent-<id>` branch whose ledger holds lines hashed
    /// under more than one salt: only the local salt's spelling of the branch can be computed, so
    /// lines under the others can't be selected. Refused rather than reported short
    /// (Governance II).
    #[error(
        "can't select {branch} from a ledger hashed under {salts} salts: agent branches are \
         stored hashed, and lines under a salt other than the local one can't be found"
    )]
    AgentBranchAcrossSalts {
        /// The requested agent branch.
        branch: String,
        /// How many salt fingerprints the ledger's redacted lines carry.
        salts: u64,
    },
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
    /// Calls with no branch stamp that no branch was assigned, over the full attributed set.
    unstamped: crate::model::Unstamped,
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
    let unstamped = crate::summary::count_unstamped(&attributed, mode);
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
        unstamped,
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
    summary.unstamped = sel.unstamped;
    // A saturated count is a floor, so the totals are unreliable (#96). `summarize` already
    // covers the unattributed count it was handed.
    if sel.unstamped.calls == u64::MAX || sel.unstamped.tokens == u64::MAX {
        summary.footnote.totals_overflowed = true;
    }
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
        pending_unavailable: None,
        pending_ledger_stats: None,
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
    let sel = select_for(
        live.records,
        &live.dispatches,
        live.captures.events(),
        &live.main_checkout,
        &live.worktrees,
        &branch,
        options,
    );
    // Pending lines outside what this report counts (decision 0006). An unreadable ledger leaves
    // them unknown: the report proceeds with zero and says so, never reads it as "none".
    let (pending, pending_ledger_stats, pending_unavailable) =
        match pending::live_pending(git, repo_dir, &live.main_checkout, &sel.selected) {
            Ok((p, stats)) => (p, stats, None),
            Err(e) => (crate::model::Pending::default(), None, Some(e)),
        };
    // Nothing counted is an error, unless pending lines remain: those are reported, not hidden.
    if sel.selected.is_empty() && pending.lines == 0 {
        return Err(ReportError::NoRecords {
            branch,
            read_stats: live.stats,
        });
    }
    let mut report = finish(&branch, options, sel, live.stats, table);
    report.worktrees_unavailable = live.unavailable;
    report.pending_unavailable = pending_unavailable;
    report.pending_ledger_stats = pending_ledger_stats;
    report.capture_stats = live.captures.stats().clone();
    report.summary.pending = pending;
    Ok(report)
}

/// The ledger's entries for one requested branch. An agent branch (`worktree-agent-<id>`) is
/// stored under its hashed name on redacted lines, so the raw name and the hashed name are both
/// read: a ledger that spells it both ways is the mixed case, counted and noted, not hidden.
struct BranchLedger {
    entries: Vec<crate::ledger::LedgerEntry>,
    /// `(hashed, requested)`: a record's branch, hashed on a redacted line, reads as requested.
    alias: Option<(String, String)>,
    /// Lines dropped because the same request id was held under both branch spellings.
    collapsed: u64,
}

/// Which of two lines for one request id to keep, by the ledger's own conflict rule: the latest
/// `recorded_at` (a missing one is oldest), then the smallest serialized line.
fn newer_line(a: &crate::ledger::LedgerEntry, b: &crate::ledger::LedgerEntry) -> bool {
    let line = |e: &crate::ledger::LedgerEntry| serde_json::to_string(e).unwrap_or_default();
    match a.recorded_at.cmp(&b.recorded_at) {
        std::cmp::Ordering::Equal => line(a) < line(b),
        later => later == std::cmp::Ordering::Greater,
    }
}

impl BranchLedger {
    fn new(
        read: &crate::ledger::Ledger,
        branch: &str,
        redactor: Option<&crate::ledger::KeyedRedact>,
    ) -> Self {
        let mut halves = vec![read.for_branch(branch)];
        let mut alias = None;
        if let Some(red) = redactor {
            let hashed = red.redact_branch(branch);
            if hashed != branch {
                halves.push(read.for_branch(&hashed));
                alias = Some((hashed, branch.to_owned()));
            }
        }
        // One call under both spellings is one call: keep one line per request id, in first-seen
        // order, and count the rest.
        let mut entries: Vec<crate::ledger::LedgerEntry> = Vec::new();
        let mut at: std::collections::HashMap<crate::model::RequestId, usize> =
            std::collections::HashMap::new();
        let mut collapsed = 0;
        // A pending line is in no branch's totals (decision 0006): it is reported only as pending.
        for entry in halves
            .iter()
            .flat_map(crate::ledger::Ledger::entries)
            .filter(|e| !e.pending)
        {
            if let Some(&i) = at.get(&entry.request_id) {
                collapsed += 1;
                if newer_line(entry, &entries[i]) {
                    entries[i] = entry.clone();
                }
            } else {
                at.insert(entry.request_id.clone(), entries.len());
                entries.push(entry.clone());
            }
        }
        Self {
            entries,
            alias,
            collapsed,
        }
    }

    fn records(&self, main: &Path) -> Vec<crate::model::SelectedRecord> {
        let mut out: Vec<_> = self
            .entries
            .iter()
            .map(|e| e.to_selected(Some(main)))
            .collect();
        if let Some((hashed, requested)) = &self.alias {
            for r in &mut out {
                if &r.attributed.branch == hashed {
                    r.attributed.branch.clone_from(requested);
                }
            }
        }
        out
    }

    fn held(&self) -> std::collections::HashSet<crate::model::RequestId> {
        self.entries.iter().map(|e| e.request_id.clone()).collect()
    }

    fn modes(&self) -> Vec<Mode> {
        [Mode::Strict, Mode::Claim, Mode::Timeline]
            .into_iter()
            .filter(|m| self.entries.iter().any(|e| e.mode == *m))
            .collect()
    }

    /// The ledger's records, then live records it lacks, deduplicated by request id.
    fn union(&self, live: &[crate::model::SelectedRecord], main: &Path) -> crate::ledger::Union {
        let mut records = self.records(main);
        let held = self.held();
        let ledger_len = records.len() as u64;
        let mut seen = held.clone();
        let (mut live_only, mut in_both) = (0, 0);
        let mut counted_both = std::collections::HashSet::new();
        for rec in live {
            let id = &rec.attributed.record.request_id;
            if seen.insert(id.clone()) {
                live_only += 1;
                records.push(rec.clone());
            } else if held.contains(id) && counted_both.insert(id.clone()) {
                in_both += 1;
            }
        }
        crate::ledger::Union {
            records,
            ledger_only: ledger_len - in_both,
            live_only,
            in_both,
        }
    }
}

/// Why the footnote carries a double-count risk, from what reading the ledger counted and how
/// well live calls matched it. The one place `Footnote::double_count` is set.
fn double_count_risk(
    stats: &crate::ledger::ReadStats,
    matching: LiveRedactionMatch,
) -> Option<crate::model::DoubleCountRisk> {
    use crate::model::{LiveMatchFailure, MixedLedger};
    let mixed = stats.is_mixed().then_some(MixedLedger {
        unredacted_lines: stats.unredacted_lines,
        redacted_lines: stats.redacted_lines,
        distinct_salts: stats.redaction_fingerprints.len() as u64,
    });
    let failure = match matching {
        LiveRedactionMatch::NotNeeded | LiveRedactionMatch::Matched => None,
        LiveRedactionMatch::NoSalt => Some(LiveMatchFailure::NoSalt),
        LiveRedactionMatch::SaltUnreadable => Some(LiveMatchFailure::SaltUnreadable),
        LiveRedactionMatch::FingerprintMismatch => Some(LiveMatchFailure::FingerprintMismatch),
        LiveRedactionMatch::PartiallyMatched => Some(LiveMatchFailure::PartiallyMatched),
    };
    crate::model::DoubleCountRisk::new(mixed, failure)
}

/// A ledger-only selection: the repository-wide unattributed counts are unknown, not zero.
fn ledger_only_picked(ledger_records: Vec<crate::model::SelectedRecord>) -> Picked {
    let attributed: Vec<_> = ledger_records
        .iter()
        .map(|r| r.attributed.clone())
        .collect();
    let ambiguous = crate::summary::count_ambiguous(&attributed, &ledger_records);
    Picked {
        attributed,
        selected: ledger_records,
        unattributed: (0, 0),
        unstamped: crate::model::Unstamped::default(),
        ambiguous,
        work_rule: None,
    }
}

/// The live selected records, with each one a redacted ledger line holds replaced by its hashed
/// form so the union sees one call. A call is matched by its raw id when an unredacted line holds
/// it, else by its hashed id when a redacted line does; one held in neither form is unchanged.
fn match_live_selected(
    redactor: Option<&crate::ledger::KeyedRedact>,
    held: &std::collections::HashSet<crate::model::RequestId>,
    live: Vec<crate::model::SelectedRecord>,
    main: &Path,
) -> Vec<crate::model::SelectedRecord> {
    let Some(redactor) = redactor else {
        return live;
    };
    live.into_iter()
        .map(|rec| {
            let id = &rec.attributed.record.request_id;
            if !held.contains(id) && held.contains(&redactor.salt().request_id(id)) {
                redactor.redact_selected(rec, Some(main))
            } else {
                rec
            }
        })
        .collect()
}

/// The repository-wide attributed set: every live call, plus the ledger entries live no longer
/// holds. With a redactor, a ledger entry is the same call as its live twin under the hashed id,
/// and a live call in a session the report holds only in hashed form is compared in hashed form,
/// so the ambiguity count does not miss it. A session held raw keeps its raw form.
fn full_attributed_set(
    redactor: Option<&crate::ledger::KeyedRedact>,
    live: Vec<crate::model::AttributedRecord>,
    records: &[crate::model::SelectedRecord],
    main: &Path,
) -> Vec<crate::model::AttributedRecord> {
    let mut live_ids: std::collections::HashSet<_> =
        live.iter().map(|a| a.record.request_id.clone()).collect();
    let mut all = live;
    if let Some(redactor) = redactor {
        live_ids.extend(
            all.iter()
                .map(|a| redactor.salt().request_id(&a.record.request_id)),
        );
        let sessions: std::collections::HashSet<_> = records
            .iter()
            .map(|r| r.attributed.record.session_id.clone())
            .collect();
        for a in &mut all {
            let raw = &a.record.session_id;
            if !sessions.contains(raw) && sessions.contains(&redactor.salt().session_id(raw)) {
                let wrapped = crate::model::SelectedRecord {
                    attributed: a.clone(),
                    kept_by: crate::model::Selection::Exact,
                };
                *a = redactor.redact_selected(wrapped, Some(main)).attributed;
            }
        }
    }
    all.extend(
        records
            .iter()
            .filter(|r| !live_ids.contains(&r.attributed.record.request_id))
            .map(|r| r.attributed.clone()),
    );
    all
}

/// The checkouts whose `.tally/` may hold the salt for the ledger being read, in order: the git
/// top level for the default ledger (or the checkout owning an explicit ledger in a `.tally/`),
/// then the main checkout.
fn salt_homes(ledger_root: Option<&Path>, ledger_path: &Path, main: &Path) -> Vec<PathBuf> {
    let own = match ledger_root {
        Some(root) => Some(root.to_path_buf()),
        None => ledger_path
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == crate::ledger::TALLY_DIR))
            .and_then(Path::parent)
            .map(Path::to_path_buf),
    };
    own.into_iter().chain([main.to_path_buf()]).collect()
}

/// The redactor to match live calls against redacted ledger lines, and how well it matches.
///
/// `homes` are the checkouts whose `.tally/` may hold the salt, in order of preference.
///
/// Never guesses: without a readable salt whose fingerprint is on a redacted line, there is no
/// redactor and the condition is reported. Names are not hashed here, since the union keys on
/// request ids alone.
fn live_redactor(
    homes: &[PathBuf],
    stats: &crate::ledger::ReadStats,
) -> (Option<crate::ledger::KeyedRedact>, LiveRedactionMatch) {
    use crate::ledger::{KeyedRedact, Salt, TallyDir};
    if stats.redacted_lines == 0 {
        return (None, LiveRedactionMatch::NotNeeded);
    }
    // The first `.tally/` whose salt is on the ledger's lines wins: the ledger's own, then the
    // main checkout's. An explicit ledger is hashed with the main checkout's salt even when its own
    // `.tally/` holds another, so the fingerprint, not the order, decides. Only loaded, never
    // created.
    let (mut found, mut unreadable, mut mismatched) = (None, false, false);
    for home in homes {
        match TallyDir::open(home).and_then(|t| match t {
            Some(t) => Salt::load(&t),
            None => Ok(None),
        }) {
            Ok(Some(salt)) if stats.redaction_fingerprints.contains(&salt.fingerprint()) => {
                found = Some(salt);
                break;
            }
            Ok(Some(_)) => mismatched = true,
            Ok(None) => {}
            Err(_) => unreadable = true,
        }
    }
    let Some(salt) = found else {
        let why = if unreadable {
            LiveRedactionMatch::SaltUnreadable
        } else if mismatched {
            LiveRedactionMatch::FingerprintMismatch
        } else {
            LiveRedactionMatch::NoSalt
        };
        return (None, why);
    };
    let matching = if stats.redaction_fingerprints.len() > 1 {
        LiveRedactionMatch::PartiallyMatched
    } else {
        LiveRedactionMatch::Matched
    };
    (Some(KeyedRedact::new(salt, false)), matching)
}

/// Without the salt an agent branch's hashed spelling is unknown, so its redacted lines can't be
/// selected. With live transcripts the gap is noted as a live-match failure; ledger-only it would
/// be a short total presented as complete, so it is refused (Governance II).
fn refuse_unselectable_agent_branch(
    branch: &str,
    live: bool,
    salted: bool,
    stats: &crate::ledger::ReadStats,
) -> Result<(), ReportError> {
    if live || stats.redacted_lines == 0 || !crate::ledger::is_agent_branch(branch) {
        return Ok(());
    }
    if !salted {
        return Err(ReportError::AgentBranchNeedsSalt {
            branch: branch.to_owned(),
        });
    }
    // One salt gives one hashed spelling; lines under any other salt stay unread.
    let salts = stats.redaction_fingerprints.len() as u64;
    if salts > 1 {
        return Err(ReportError::AgentBranchAcrossSalts {
            branch: branch.to_owned(),
            salts,
        });
    }
    Ok(())
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
    redaction: RecordRedaction,
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
        redaction,
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
    redaction: RecordRedaction,
    clock: &dyn crate::ledger::Clock,
) -> Result<Recorded, ReportError> {
    let branch = branch_or_current(git, repo_dir, branch)?;
    let live = read_live(git, source, projects_root, repo_dir)?;
    let mut storage = None;
    let options = resolve_options(git, &live.main_checkout, options, &mut storage);
    let default_branch = resolve_default_branch(git, &live.main_checkout);
    // Decision 0006: a line recorded on the default branch is pending until a branch is cut. On any
    // other branch, the evidence for resolving is kept before `select_for` takes the records.
    let pending = branch == default_branch.name;
    let evidence_records = (!pending).then(|| live.records.clone());
    let sel = select_for(
        live.records,
        &live.dispatches,
        live.captures.events(),
        &live.main_checkout,
        &live.worktrees,
        &branch,
        options,
    );
    let outcome = append_selected(
        git,
        repo_dir,
        target,
        redaction,
        &Append {
            records: &sel.selected,
            branch: &branch,
            mode: options.mode,
            main: &live.main_checkout,
            pending,
        },
        clock,
    )?;
    let resolution = match evidence_records {
        Some(records) => Some(
            pending::Resolving {
                git,
                repo_dir,
                target,
                clock,
                branch: &branch,
                default_branch: &default_branch.name,
                dry_run: false,
            }
            .run(
                records,
                &live.dispatches,
                live.captures.events(),
                &live.main_checkout,
                &live.worktrees,
            )?,
        ),
        None => None,
    };
    Ok(Recorded {
        pending_written: if pending { outcome.appended } else { 0 },
        resolutions_appended: resolution.as_ref().map_or(0, |r| r.appended.appended),
        resolution: resolution.map(|r| r.stats),
        branch,
        outcome,
        selected: sel.selected.len() as u64,
        read_stats: live.stats,
        worktrees_unavailable: live.unavailable,
        capture_stats: live.captures.stats().clone(),
    })
}

/// What [`record_at`] appends.
struct Append<'a> {
    records: &'a [crate::model::SelectedRecord],
    branch: &'a str,
    mode: Mode,
    main: &'a Path,
    /// The branch is the repository's default branch: write pending lines (decision 0006).
    pending: bool,
}

/// Appends to the target. The ledger is per checkout: a linked worktree records on its own
/// branch (decision 0002), so the default root is the git top level, as `--ledger`'s default is.
/// Outside a repository that is an error, never a fallback.
///
/// Keyed redaction never receives a salt: the ledger resolves it from the `.tally/` it opens, and
/// only when a line will be appended. An explicit `--ledger` takes it from the main checkout's
/// `.tally/` (decision 0005), which must exist.
fn append_selected(
    git: &dyn Git,
    repo_dir: &Path,
    target: &LedgerTarget<'_>,
    redaction: RecordRedaction,
    a: &Append<'_>,
    clock: &dyn crate::ledger::Clock,
) -> Result<crate::ledger::AppendOutcome, ReportError> {
    use crate::ledger::{
        TallyDir, append_default_pending, append_keyed_default_pending,
        append_keyed_explicit_pending, append_pending,
    };
    let main = Some(a.main);
    Ok(match (target, redaction) {
        (LedgerTarget::Default, RecordRedaction::Off) => append_default_pending(
            &git.toplevel(repo_dir)?,
            a.records,
            a.branch,
            a.mode,
            main,
            &crate::ledger::Identity,
            clock,
            a.pending,
        )?,
        (LedgerTarget::Default, RecordRedaction::Keyed { names }) => {
            let root = git.toplevel(repo_dir)?;
            let same = root == a.main
                || matches!(
                    (std::fs::canonicalize(&root), std::fs::canonicalize(a.main)),
                    (Ok(x), Ok(y)) if x == y
                );
            append_keyed_default_pending(
                &root,
                (!same).then_some(a.main),
                names,
                a.records,
                a.branch,
                a.mode,
                main,
                clock,
                a.pending,
            )?
        }
        (LedgerTarget::Explicit(path), RecordRedaction::Off) => append_pending(
            path,
            a.records,
            a.branch,
            a.mode,
            main,
            &crate::ledger::Identity,
            clock,
            a.pending,
        )?,
        (LedgerTarget::Explicit(path), RecordRedaction::Keyed { names }) => {
            let home = TallyDir::open(a.main)?.ok_or(ReportError::NoSaltHome)?;
            append_keyed_explicit_pending(
                path, &home, names, a.records, a.branch, a.mode, main, clock, a.pending,
            )?
        }
    })
}

/// The ledger a target names: the git top level (when default; a linked worktree reads its own)
/// and the file path. An explicit path has no root.
fn ledger_location(
    git: &dyn Git,
    repo_dir: &Path,
    target: &LedgerTarget<'_>,
) -> Result<(Option<PathBuf>, PathBuf), GitError> {
    Ok(match target {
        LedgerTarget::Explicit(path) => (None, path.to_path_buf()),
        LedgerTarget::Default => {
            let root = git.toplevel(repo_dir)?;
            let path = root.join(crate::ledger::LEDGER_PATH);
            (Some(root), path)
        }
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
    let (ledger_root, ledger_path) = ledger_location(git, repo_dir, target)?;
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
    let homes = salt_homes(ledger_root.as_deref(), &ledger_path, &main_checkout);
    let (redactor, matching) = live_redactor(&homes, read.stats());
    refuse_unselectable_agent_branch(&branch, source.is_some(), redactor.is_some(), read.stats())?;
    // Exactly the entries `record` wrote for this branch; the ledger keys on (request, branch).
    let ledger = BranchLedger::new(&read, &branch, redactor.as_ref());
    let ledger_records = ledger.records(&main_checkout);
    let mut info = LedgerReport {
        ledger_stats: read.stats().clone(),
        ledger_only: ledger_records.len() as u64,
        live_only: 0,
        in_both: 0,
        modes: ledger.modes(),
        live_included: source.is_some(),
        full_set: FullSetCounts::Unavailable,
        live_read_stats: None,
        collapsed_lines: ledger.collapsed,
        live_redaction: if source.is_some() {
            matching
        } else {
            LiveRedactionMatch::NotNeeded
        },
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
        // Against redacted lines, live calls are matched in hashed form (decision 0005).
        let live_selected = match_live_selected(
            redactor.as_ref(),
            &ledger.held(),
            live_sel.selected,
            &main_checkout,
        );
        // The ledger's copy wins on overlap.
        let union = ledger.union(&live_selected, &main_checkout);
        let records = union.records;
        info.ledger_only = union.ledger_only;
        info.live_only = union.live_only;
        info.in_both = union.in_both;
        // Repository-wide counts: the live full set, plus ledger entries live no longer holds.
        let all = full_attributed_set(
            redactor.as_ref(),
            live_sel.attributed,
            &records,
            &main_checkout,
        );
        info.full_set = FullSetCounts::FromLive;
        info.live_read_stats = Some(live.stats);
        let sel = Picked {
            unattributed: crate::model::SubagentSummary::count_unattributed(&all),
            // Live records only: ledger entries were selected for their branch, so are placed.
            unstamped: live_sel.unstamped,
            ambiguous: crate::summary::count_ambiguous(&all, &records),
            attributed: all,
            selected: records,
            work_rule: live_sel.work_rule,
        };
        (sel, live.stats)
    } else {
        (ledger_only_picked(ledger_records), ReadStats::default())
    };
    // Repository-wide, not this branch's: pending lines wait for a branch that is not this one
    // yet. A request the totals already count is not also pending.
    let salts = pending::matching_salts(&homes, &read);
    let counted = pending::stored_forms(
        sel.selected.iter().map(|s| &s.attributed.record.request_id),
        &salts,
    );
    let pending = pending::pending_outside(&read, &counted);
    // Nothing counted is an error, unless pending lines remain: those are reported, not hidden.
    if sel.selected.is_empty() && pending.lines == 0 {
        return Err(ReportError::NoRecords { branch, read_stats });
    }
    let mut report = finish(&branch, options, sel, read_stats, table);
    report.worktrees_unavailable = unavailable;
    report.capture_stats = capture_stats;
    report.summary.footnote.double_count = double_count_risk(read.stats(), info.live_redaction);
    report.summary.pending = pending;
    report.ledger = Some(info);
    Ok(report)
}
