//! Integration tests for hook captures in `report::run` and `report::record` (#13): fake git and
//! source, a captures file in a temp repository. No transcript content is involved.

#![expect(clippy::unwrap_used, reason = "tests fail loudly on a wrong result")]

use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::ledger::{
    CaptureReadStats, Clock, Identity, append_captures, captures_path,
};
use branch_tally_core::model::{
    BranchBasis, CAPTURE_SCHEMA_VERSION, CaptureEvent, CaptureKind, Dispatch, Harness, Origin,
    Record, RequestId, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::report::{Options, Report, ReportError, record, run};
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
    let git = FakeGit::new().with_worktrees(
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
