//! Integration tests for hook captures in `report::run` and `report::record` (#13): fake git and
//! source, a captures file in a temp repository. No transcript content is involved.

#![expect(clippy::unwrap_used, reason = "tests fail loudly on a wrong result")]

use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::ledger::{
    CaptureReadStats, Clock, Identity, LedgerTarget, append_captures, captures_path,
};
use branch_tally_core::model::{
    BranchBasis, CAPTURE_SCHEMA_VERSION, CaptureEvent, CaptureKind, Dispatch, Harness, Origin,
    Record, RequestId, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::report::{
    Options, Report, ReportError, record, record_at, run, run_from_ledger_at,
};
use branch_tally_core::sources::{
    ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot,
};

fn rec(request: &str, branch: Option<&str>, ts: &str) -> Record {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new("s"),
        RequestId::new(request),
        Usage {
            input: 10,
            output: 5,
            ..Usage::default()
        },
    );
    r.model = Some("m".to_owned());
    r.git_branch = branch.map(str::to_owned);
    r.origin = Some(Origin::Main);
    r.ts = Some(ts.to_owned());
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

struct Tick;

impl Clock for Tick {
    fn now(&self) -> String {
        "2026-11-01T13:00:00Z".to_owned()
    }
}

fn cap_event(kind: CaptureKind, ts: &str) -> CaptureEvent {
    CaptureEvent {
        v: CAPTURE_SCHEMA_VERSION,
        harness: Harness::ClaudeCode,
        kind,
        session_id: SessionId::new("s"),
        agent_id: None,
        branch: "feat/cap".to_owned(),
        worktree_branch: None,
        ts: ts.to_owned(),
    }
}

/// A temp "repository" that is its own main checkout with only the main worktree left (the agent
/// worktree is gone), and two records: one stamped with a vanished `worktree-agent-<id>`, one
/// unstamped.
fn setup() -> (tempfile::TempDir, FakeGit, FakeSource) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    // Captures are written only where the repository has opted in by creating `.tally/` (#94).
    std::fs::create_dir(root.join(".tally")).unwrap();
    let git = FakeGit::new().with_toplevel(root.clone(), root.clone());
    let git = git.with_worktrees(
        root.clone(),
        vec![Worktree {
            path: root,
            branch: "main".to_owned(),
        }],
    );
    let source = FakeSource(vec![
        rec("a", Some("worktree-agent-abc"), "2026-11-01T11:00:00Z"),
        rec("b", None, "2026-11-01T11:30:00Z"),
    ]);
    (dir, git, source)
}

fn write_captures(root: &Path) {
    append_captures(
        root,
        &[
            cap_event(CaptureKind::SessionStart, "2026-11-01T10:00:00Z"),
            cap_event(CaptureKind::Stop, "2026-11-01T12:00:00Z"),
        ],
    )
    .unwrap();
}

fn live(
    git: &FakeGit,
    source: &FakeSource,
    root: &Path,
    branch: &str,
) -> Result<Report, ReportError> {
    run(
        git,
        source,
        Path::new("/projects"),
        root,
        Some(branch),
        &Options::default(),
        PriceTable::bundled(),
    )
}

#[test]
fn run_uses_captures_for_vanished_worktree_and_unstamped_records_and_says_so() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    let report = live(&git, &source, dir.path(), "feat/cap").unwrap();
    assert_eq!(report.summary.footnote.heuristics.captured.calls, 2);
    assert_eq!(report.capture_stats.events, 2);
    assert_eq!(report.capture_stats.malformed, 0);
}

#[test]
fn run_without_a_captures_file_changes_nothing() {
    let (dir, git, source) = setup();
    // Neither record can be placed on feat/cap without captures.
    assert!(matches!(
        live(&git, &source, dir.path(), "feat/cap"),
        Err(ReportError::NoRecords { .. })
    ));
    let r = live(&git, &source, dir.path(), "worktree-agent-abc").unwrap();
    assert_eq!(r.summary.footnote.heuristics.captured.calls, 0);
    assert_eq!(r.capture_stats, CaptureReadStats::default());
}

#[test]
fn bad_capture_lines_are_counted_on_the_report() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    let path = captures_path(dir.path());
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("not json\n");
    std::fs::write(&path, text).unwrap();
    let report = live(&git, &source, dir.path(), "feat/cap").unwrap();
    assert_eq!(report.capture_stats.malformed, 1);
    assert_eq!(report.capture_stats.bad_line_numbers, vec![3]);
    assert_eq!(report.summary.footnote.heuristics.captured.calls, 2);
}

#[test]
fn record_uses_captures_and_reports_their_stats() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    let ledger: PathBuf = dir.path().join(".tally").join("ledger.jsonl");
    let recorded = record(
        &git,
        &source,
        Path::new("/projects"),
        dir.path(),
        Some("feat/cap"),
        &Options::default(),
        &ledger,
        &Identity,
        &Tick,
    )
    .unwrap();
    assert_eq!(recorded.selected, 2);
    assert_eq!(recorded.capture_stats.events, 2);
    let read = branch_tally_core::ledger::read(&ledger).unwrap();
    let selected = read.for_branch("feat/cap").to_selected(Some(dir.path()));
    assert_eq!(selected.len(), 2);
    assert!(
        selected
            .iter()
            .all(|s| s.attributed.basis == BranchBasis::Captured)
    );
}

// The default `.tally/` is guarded (#94): `record_at` and `run_from_ledger_at` with
// `LedgerTarget::Default` write only where `.tally/` exists and refuse planted links.

fn record_default(
    git: &FakeGit,
    source: &FakeSource,
    root: &Path,
) -> Result<branch_tally_core::report::Recorded, ReportError> {
    record_at(
        git,
        source,
        Path::new("/projects"),
        root,
        Some("feat/cap"),
        &Options::default(),
        &LedgerTarget::Default,
        &Identity,
        &Tick,
    )
}

fn report_default(
    git: &FakeGit,
    source: Option<&FakeSource>,
    root: &Path,
) -> Result<Report, ReportError> {
    run_from_ledger_at(
        git,
        source.map(|s| s as &dyn Source),
        Path::new("/projects"),
        root,
        Some("feat/cap"),
        &Options::default(),
        &LedgerTarget::Default,
        PriceTable::bundled(),
    )
}

#[test]
fn default_record_writes_the_ledger_under_an_existing_tally_dir_and_reads_it_back() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    let recorded = record_default(&git, &source, dir.path()).unwrap();
    assert_eq!(recorded.outcome.appended, 2);
    assert!(!recorded.outcome.tally_absent);
    assert!(dir.path().join(".tally").join("ledger.jsonl").is_file());
    let report = report_default(&git, None, dir.path()).unwrap();
    assert_eq!(report.ledger.unwrap().ledger_only, 2);
}

#[test]
fn default_record_with_no_tally_dir_does_nothing_and_says_so() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    std::fs::remove_dir_all(dir.path().join(".tally")).unwrap();
    let recorded = record_default(&git, &source, dir.path()).unwrap();
    assert_eq!(recorded.outcome.appended, 0);
    assert!(recorded.outcome.tally_absent);
    assert!(!dir.path().join(".tally").exists());
}

#[test]
fn default_ledger_from_a_linked_worktree_is_the_worktrees_own_toplevel_ledger() {
    // Captures live in the main checkout; the ledger is per checkout (decision 0002).
    let (main, git, source) = setup();
    write_captures(main.path());
    let wt = tempfile::tempdir().unwrap();
    std::fs::create_dir(wt.path().join(".tally")).unwrap();
    let git = git
        .with_main_checkout(wt.path().to_owned(), main.path().to_owned())
        .with_toplevel(wt.path().to_owned(), wt.path().to_owned());
    let recorded = record_default(&git, &source, wt.path()).unwrap();
    assert_eq!(recorded.outcome.appended, 2);
    let ledger = |root: &Path| root.join(".tally").join("ledger.jsonl");
    assert!(ledger(wt.path()).is_file());
    assert!(!ledger(main.path()).exists());
    let report = report_default(&git, None, wt.path()).unwrap();
    assert_eq!(report.ledger.unwrap().ledger_only, 2);
}

#[test]
fn default_ledger_outside_a_checkout_is_a_git_error_not_a_fallback() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    let stray = tempfile::tempdir().unwrap();
    let err = record_default(&git, &source, stray.path()).unwrap_err();
    assert!(matches!(err, ReportError::Git(_)), "{err}");
}

#[cfg(unix)]
#[test]
fn default_record_refuses_a_planted_ledger_symlink() {
    let (dir, git, source) = setup();
    write_captures(dir.path());
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"precious").unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join(".tally").join("ledger.jsonl")).unwrap();
    let err = record_default(&git, &source, dir.path()).unwrap_err();
    assert!(
        matches!(
            err,
            ReportError::Ledger(branch_tally_core::ledger::LedgerError::Refused { .. })
        ),
        "{err}"
    );
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    assert!(report_default(&git, None, dir.path()).is_err());
}

#[cfg(unix)]
#[test]
fn a_planted_captures_symlink_is_an_error_not_an_empty_read() {
    let (dir, git, source) = setup();
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"precious").unwrap();
    std::os::unix::fs::symlink(&victim, captures_path(dir.path())).unwrap();
    assert!(live(&git, &source, dir.path(), "feat/cap").is_err());
    assert!(record_default(&git, &source, dir.path()).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
}
