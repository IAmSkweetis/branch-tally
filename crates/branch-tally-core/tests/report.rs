//! Integration tests for `report`, built from in-code records and fakes (no transcript content).

#![expect(clippy::panic, reason = "a wrong error variant fails the test loudly")]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::model::{Harness, Mode, Origin, Record, RequestId, SessionId, Usage};
use branch_tally_core::report::{ReportError, build, run};
use branch_tally_core::sources::{ReadStats, RepoIdentity, Source, SourceError, TranscriptRoot};

fn rec(session: &str, request: &str, branch: Option<&str>) -> Record {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
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
    r
}

struct FakeSource {
    records: Vec<Record>,
    stats: ReadStats,
    repo_seen: RefCell<Option<PathBuf>>,
}

impl FakeSource {
    fn new(records: Vec<Record>, stats: ReadStats) -> Self {
        Self {
            records,
            stats,
            repo_seen: RefCell::new(None),
        }
    }
}

impl Source for FakeSource {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }
    fn discover(
        &self,
        _projects_root: &Path,
        repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError> {
        *self.repo_seen.borrow_mut() = Some(repo.root().to_path_buf());
        Ok(Vec::new())
    }
    fn read(
        &self,
        _roots: &[TranscriptRoot],
        sink: &mut dyn FnMut(Record),
    ) -> Result<ReadStats, SourceError> {
        for r in &self.records {
            sink(r.clone());
        }
        Ok(self.stats)
    }
}

fn stats() -> ReadStats {
    ReadStats {
        malformed_lines: 3,
        ..ReadStats::default()
    }
}

#[test]
fn build_selects_summarizes_and_carries_read_stats() {
    let records = vec![
        rec("s1", "a", Some("feat/x")),
        rec("s1", "b", Some("feat/x")),
        rec("s2", "c", Some("other")),
    ];
    let report = build(records, stats(), Path::new("/repo"), &[], "feat/x", false).unwrap();
    assert_eq!(report.branch, "feat/x");
    assert_eq!(report.summary.mode, Mode::Claim);
    assert_eq!(report.summary.total.calls, 2);
    assert_eq!(report.read_stats.malformed_lines, 3);
    assert!(report.worktrees_unavailable.is_none());
}

#[test]
fn build_reports_strict_mode() {
    let records = vec![rec("s1", "a", Some("feat/x"))];
    let report = build(records, stats(), Path::new("/repo"), &[], "feat/x", true).unwrap();
    assert_eq!(report.summary.mode, Mode::Strict);
}

#[test]
fn build_with_nothing_selected_is_an_explicit_outcome_with_stats() {
    let records = vec![rec("s1", "a", Some("other"))];
    let err = build(records, stats(), Path::new("/repo"), &[], "feat/x", false).unwrap_err();
    match err {
        ReportError::NoRecords { branch, read_stats } => {
            assert_eq!(branch, "feat/x");
            assert_eq!(read_stats.malformed_lines, 3);
        }
        other => panic!("expected NoRecords, got {other}"),
    }
}

#[test]
fn run_defaults_to_the_current_branch_and_reads_for_the_main_checkout() {
    let git = FakeGit::new()
        .with_branch("/repo/wt", "feat/x")
        .with_main_checkout("/repo/wt", "/repo")
        .with_worktrees(
            "/repo",
            vec![Worktree {
                path: PathBuf::from("/repo"),
                branch: "main".to_owned(),
            }],
        );
    let source = FakeSource::new(vec![rec("s1", "a", Some("feat/x"))], stats());
    let report = run(
        &git,
        &source,
        Path::new("/projects"),
        Path::new("/repo/wt"),
        None,
        false,
    )
    .unwrap();
    assert_eq!(report.branch, "feat/x");
    assert_eq!(
        source.repo_seen.borrow().as_deref(),
        Some(Path::new("/repo"))
    );
    assert!(report.worktrees_unavailable.is_none());
}

#[test]
fn run_treats_an_unreadable_worktree_list_as_empty_but_says_so() {
    // No worktrees scripted: the fake's `worktrees` fails, as git can.
    let git = FakeGit::new();
    let source = FakeSource::new(vec![rec("s1", "a", Some("feat/x"))], stats());
    let report = run(
        &git,
        &source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some("feat/x"),
        false,
    )
    .unwrap();
    assert_eq!(report.summary.total.calls, 1);
    assert!(report.worktrees_unavailable.is_some());
}

#[test]
fn run_without_a_branch_and_without_git_is_a_git_error() {
    let git = FakeGit::new();
    let source = FakeSource::new(Vec::new(), ReadStats::default());
    let err = run(
        &git,
        &source,
        Path::new("/projects"),
        Path::new("/repo"),
        None,
        false,
    )
    .unwrap_err();
    assert!(matches!(err, ReportError::Git(_)), "{err}");
}
