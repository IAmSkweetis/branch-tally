//! Integration tests for `record` and `run_from_ledger` (#8): in-code records and real fixtures,
//! with tempdir ledgers. No transcript content is involved.

#![expect(clippy::unwrap_used, reason = "tests fail loudly on a wrong result")]

use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::ledger::{Clock, Identity};
use branch_tally_core::model::{
    Dispatch, Harness, Mode, Origin, Record, RequestId, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::report::{
    FullSetCounts, Options, Report, ReportError, record, run, run_from_ledger,
};
use branch_tally_core::sources::claude_code::ClaudeCode;
use branch_tally_core::sources::{
    ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot,
};

fn rec(session: &str, request: &str, branch: &str, output: u64) -> Record {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
        RequestId::new(request),
        Usage {
            input: 10,
            output,
            ..Usage::default()
        },
    );
    r.model = Some("m".to_owned());
    r.git_branch = Some(branch.to_owned());
    r.origin = Some(Origin::Main);
    r
}

struct FakeSource(Vec<Record>);

impl Source for FakeSource {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }
    fn discover(
        &self,
        _projects_root: &Path,
        _repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError> {
        Ok(vec![TranscriptRoot::new(
            "/projects/-repo",
            RootKind::Repository,
        )])
    }
    fn read(
        &self,
        _roots: &[TranscriptRoot],
        sink: &mut dyn FnMut(Record),
    ) -> Result<ReadStats, SourceError> {
        for r in &self.0 {
            sink(r.clone());
        }
        Ok(ReadStats::default())
    }
    fn dispatches(&self, _roots: &[TranscriptRoot]) -> Result<Vec<Dispatch>, SourceError> {
        Ok(Vec::new())
    }
}

struct FixedClock(&'static str);

impl Clock for FixedClock {
    fn now(&self) -> String {
        self.0.to_owned()
    }
}

fn git() -> FakeGit {
    FakeGit::new().with_worktrees(
        "/repo",
        vec![Worktree {
            path: PathBuf::from("/repo"),
            branch: "main".to_owned(),
        }],
    )
}

fn table() -> &'static PriceTable {
    PriceTable::bundled()
}

fn do_record(
    source: &dyn Source,
    branch: &str,
    ledger: &Path,
    at: &'static str,
) -> branch_tally_core::report::Recorded {
    record(
        &git(),
        source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some(branch),
        &Options::default(),
        ledger,
        &Identity,
        &FixedClock(at),
    )
    .unwrap()
}

fn from_ledger(
    source: Option<&dyn Source>,
    branch: &str,
    ledger: &Path,
) -> Result<Report, ReportError> {
    run_from_ledger(
        &git(),
        source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some(branch),
        &Options::default(),
        ledger,
        table(),
    )
}

fn live(source: &dyn Source, branch: &str) -> Report {
    run(
        &git(),
        source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some(branch),
        &Options::default(),
        table(),
    )
    .unwrap()
}

fn ledger_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(".tally").join("ledger.jsonl")
}

fn three_records() -> FakeSource {
    FakeSource(vec![
        rec("s1", "a", "feat/x", 1),
        rec("s1", "b", "feat/x", 2),
        rec("s2", "c", "other", 4),
    ])
}

#[test]
fn record_writes_the_branchs_records_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    let source = three_records();
    let first = do_record(&source, "feat/x", &path, "2026-01-01T00:00:00Z");
    assert_eq!(
        (
            first.outcome.appended,
            first.outcome.skipped,
            first.selected
        ),
        (2, 0, 2)
    );
    let second = do_record(&source, "feat/x", &path, "2026-01-02T00:00:00Z");
    assert_eq!(
        (second.outcome.appended, second.outcome.skipped),
        (0, 2),
        "a second record must append nothing"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
}

#[test]
fn record_carries_the_source_read_stats() {
    let dir = tempfile::tempdir().unwrap();
    let out = do_record(
        &three_records(),
        "feat/x",
        &ledger_path(&dir),
        "2026-01-01T00:00:00Z",
    );
    assert_eq!(out.branch, "feat/x");
    assert_eq!(out.read_stats, ReadStats::default());
}

#[test]
fn ledger_only_totals_equal_the_live_report_after_recording() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    let source = three_records();
    do_record(&source, "feat/x", &path, "2026-01-01T00:00:00Z");
    let from = from_ledger(None, "feat/x", &path).unwrap();
    let want = live(&source, "feat/x");
    assert_eq!(from.summary.total, want.summary.total);
    assert_eq!(from.summary.by_model, want.summary.by_model);
    assert_eq!(from.costs.total, want.costs.total);
    assert_eq!(from.summary.total.calls, 2);
}

#[test]
fn session_claimed_records_recorded_off_branch_still_count_for_the_branch() {
    // `b` is stamped `other` but its session also did work on `feat/x`, so claim mode keeps it.
    let source = FakeSource(vec![
        rec("s1", "a", "feat/x", 1),
        rec("s1", "b", "other", 2),
        rec("s2", "c", "other", 4),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(&source, "feat/x", &path, "2026-01-01T00:00:00Z");
    let from = from_ledger(None, "feat/x", &path).unwrap();
    let want = live(&source, "feat/x");
    assert_eq!(want.summary.total.calls, 2);
    assert_eq!(from.summary.total, want.summary.total);
    assert_eq!(from.summary.footnote, want.summary.footnote);
}

#[test]
fn other_branches_rows_are_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    let source = three_records();
    do_record(&source, "feat/x", &path, "2026-01-01T00:00:00Z");
    do_record(&source, "other", &path, "2026-01-01T00:00:00Z");
    let x = from_ledger(None, "feat/x", &path).unwrap();
    let other = from_ledger(None, "other", &path).unwrap();
    assert_eq!(x.summary.total.calls, 2);
    assert_eq!(other.summary.total.calls, 1);
}

#[test]
fn a_union_double_counts_nothing_when_ledger_and_live_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(
        &FakeSource(vec![
            rec("s1", "a", "feat/x", 1),
            rec("s1", "b", "feat/x", 2),
        ]),
        "feat/x",
        &path,
        "2026-01-01T00:00:00Z",
    );
    let source = FakeSource(vec![
        rec("s1", "a", "feat/x", 1),
        rec("s1", "b", "feat/x", 2),
        rec("s1", "c", "feat/x", 4),
    ]);
    let report = from_ledger(Some(&source), "feat/x", &path).unwrap();
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.total, live(&source, "feat/x").summary.total);
    let l = report.ledger.unwrap();
    assert_eq!((l.in_both, l.live_only, l.ledger_only), (2, 1, 0));
    assert!(l.live_included);
}

#[test]
fn the_ledgers_copy_wins_on_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(
        &FakeSource(vec![rec("s1", "a", "feat/x", 1)]),
        "feat/x",
        &path,
        "2026-01-01T00:00:00Z",
    );
    let source = FakeSource(vec![rec("s1", "a", "feat/x", 999)]);
    let report = from_ledger(Some(&source), "feat/x", &path).unwrap();
    assert_eq!(report.summary.total.output, 1);
}

#[test]
fn ledger_only_does_not_pretend_the_repository_wide_counts_are_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(&three_records(), "feat/x", &path, "2026-01-01T00:00:00Z");
    let report = from_ledger(None, "feat/x", &path).unwrap();
    let l = report.ledger.unwrap();
    assert_eq!(l.full_set, FullSetCounts::Unavailable);
    assert!(!l.live_included);
}

#[test]
fn a_union_takes_the_repository_wide_counts_from_the_live_side() {
    // The fixture has subagent calls with no dispatch link: live counts them unattributed.
    let repo = PathBuf::from(r"C:\repo");
    let git = FakeGit::new().with_worktrees(
        &repo,
        vec![Worktree {
            path: repo.clone(),
            branch: "main".to_owned(),
        }],
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/claude-code/subagent-no-dispatch-link");
    let opts = Options::default();
    let want = run(
        &git,
        &ClaudeCode,
        &root,
        &repo,
        Some("feat/x"),
        &opts,
        table(),
    )
    .unwrap();
    assert!(want.summary.subagents.unattributed_calls > 0);

    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    branch_tally_core::report::record(
        &git,
        &ClaudeCode,
        &root,
        &repo,
        Some("feat/x"),
        &opts,
        &path,
        &Identity,
        &FixedClock("2026-01-01T00:00:00Z"),
    )
    .unwrap();

    let only = run_from_ledger(
        &git,
        None,
        &root,
        &repo,
        Some("feat/x"),
        &opts,
        &path,
        table(),
    )
    .unwrap();
    assert_eq!(only.summary.total, want.summary.total);
    assert_eq!(only.ledger.unwrap().full_set, FullSetCounts::Unavailable);

    let both = run_from_ledger(
        &git,
        Some(&ClaudeCode),
        &root,
        &repo,
        Some("feat/x"),
        &opts,
        &path,
        table(),
    )
    .unwrap();
    assert_eq!(both.summary.total, want.summary.total);
    assert_eq!(
        both.summary.subagents.unattributed_calls,
        want.summary.subagents.unattributed_calls
    );
    assert_eq!(both.ledger.unwrap().full_set, FullSetCounts::FromLive);
}

#[test]
fn malformed_and_conflicting_ledger_lines_are_counted_on_the_report() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(&three_records(), "feat/x", &path, "2026-01-01T00:00:00Z");
    let text = std::fs::read_to_string(&path).unwrap();
    let first = text.lines().next().unwrap().to_owned();
    // The same request id, different data, later recorded_at.
    let conflicting = first
        .replace("2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z")
        .replace("\"output\":1", "\"output\":50");
    assert_ne!(conflicting, first);
    std::fs::write(&path, format!("{text}not json\n{conflicting}\n")).unwrap();
    let report = from_ledger(None, "feat/x", &path).unwrap();
    let stats = report.ledger.unwrap().ledger_stats;
    assert_eq!((stats.malformed, stats.conflicts), (1, 1));
    assert_eq!(report.summary.total.output, 52, "the later write wins");
}

#[test]
fn a_missing_ledger_without_live_is_a_typed_error_not_an_empty_table() {
    let dir = tempfile::tempdir().unwrap();
    let err = from_ledger(None, "feat/x", &ledger_path(&dir)).unwrap_err();
    assert!(matches!(err, ReportError::LedgerMissing { .. }), "{err}");
}

#[test]
fn a_missing_ledger_with_live_reports_the_live_records() {
    let dir = tempfile::tempdir().unwrap();
    let source = three_records();
    let report = from_ledger(Some(&source), "feat/x", &ledger_path(&dir)).unwrap();
    assert_eq!(report.summary.total.calls, 2);
    assert_eq!(report.ledger.unwrap().live_only, 2);
}

#[test]
fn a_ledger_with_nothing_for_the_branch_is_no_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(&three_records(), "feat/x", &path, "2026-01-01T00:00:00Z");
    let err = from_ledger(None, "nope", &path).unwrap_err();
    assert!(matches!(err, ReportError::NoRecords { .. }), "{err}");
}

#[test]
fn strict_mode_keeps_its_meaning_on_a_ledger_report() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger_path(&dir);
    do_record(&three_records(), "feat/x", &path, "2026-01-01T00:00:00Z");
    let report = run_from_ledger(
        &git(),
        None,
        Path::new("/projects"),
        Path::new("/repo"),
        Some("feat/x"),
        &Options::new(Mode::Strict),
        &path,
        table(),
    )
    .unwrap();
    assert_eq!(report.summary.mode, Mode::Strict);
}
