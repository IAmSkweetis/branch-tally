//! Pending ledger lines end to end (#131, decision 0006): `record_at` on the default branch writes
//! pending lines, `record_at` elsewhere resolves them, `resolve_at` does it on request, and the
//! report counts what is still unresolved. In-code records with synthetic ids; no content.

#![expect(clippy::unwrap_used, reason = "tests fail loudly on a wrong result")]

use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::ledger::{Clock, LedgerTarget, append_captures};
use branch_tally_core::model::{
    CAPTURE_SCHEMA_VERSION, CaptureEvent, CaptureKind, Dispatch, Harness, Mode, Origin, Record,
    RequestId, SessionId, Timestamp, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::render::{json, markdown};
use branch_tally_core::report::{
    Options, RecordRedaction, Recorded, ResolveOutcome, record_at, resolve_at, run,
    run_from_ledger_at,
};
use branch_tally_core::sources::{
    ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot,
};

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

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> String {
        "2026-01-01T00:00:00Z".to_owned()
    }
}

/// `second` of 2026-01-01T00:00:SSZ, or none.
fn at(second: Option<u32>) -> Option<Timestamp> {
    second.map(|s| format!("2026-01-01T00:00:{s:02}Z").parse().unwrap())
}

fn rec(session: &str, request: &str, branch: &str, second: Option<u32>) -> Record {
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
    r.git_branch = Some(branch.to_owned());
    r.origin = Some(Origin::Main);
    r.ts = at(second);
    r
}

struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".tally")).unwrap();
        Self { dir }
    }
    fn root(&self) -> PathBuf {
        self.dir.path().to_owned()
    }
    fn ledger(&self) -> PathBuf {
        self.dir.path().join(".tally").join("ledger.jsonl")
    }
    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.ledger())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn git(&self) -> FakeGit {
        FakeGit::new()
            .with_toplevel(self.root(), self.root())
            .with_worktrees(
                self.root(),
                vec![Worktree {
                    path: self.root(),
                    branch: "main".to_owned(),
                }],
            )
    }
    fn record(&self, branch: &str, records: Vec<Record>, redaction: RecordRedaction) -> Recorded {
        self.record_in(Mode::Strict, branch, records, redaction)
    }
    fn record_in(
        &self,
        mode: Mode,
        branch: &str,
        records: Vec<Record>,
        redaction: RecordRedaction,
    ) -> Recorded {
        record_at(
            &self.git(),
            &FakeSource(records),
            Path::new("/projects"),
            &self.root(),
            Some(branch),
            &Options::new(mode),
            &LedgerTarget::Default,
            redaction,
            &FixedClock,
        )
        .unwrap()
    }
    fn resolve(&self, branch: &str, records: Vec<Record>, dry_run: bool) -> ResolveOutcome {
        resolve_at(
            &self.git(),
            &FakeSource(records),
            Path::new("/projects"),
            &self.root(),
            Some(branch),
            &LedgerTarget::Default,
            &FixedClock,
            dry_run,
        )
        .unwrap()
    }
    fn report(&self, branch: &str, live: Option<&FakeSource>) -> branch_tally_core::report::Report {
        run_from_ledger_at(
            &self.git(),
            live.map(|s| s as &dyn Source),
            Path::new("/projects"),
            &self.root(),
            Some(branch),
            &Options::default(),
            &LedgerTarget::Default,
            PriceTable::bundled(),
        )
        .unwrap()
    }
}

/// A session that starts on `main` and cuts `feat/x`: two calls before the cut, one after.
fn cut_session() -> Vec<Record> {
    vec![
        rec("s1", "req-a", "main", Some(1)),
        rec("s1", "req-b", "main", Some(2)),
        rec("s1", "req-c", "feat/x", Some(3)),
    ]
}

#[test]
fn recording_on_the_default_branch_writes_pending_lines() {
    let repo = Repo::new();
    let r = repo.record("main", cut_session(), RecordRedaction::Off);
    assert_eq!(r.pending_written, 2);
    assert!(r.resolution.is_none());
    let lines = repo.lines();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| l.contains("\"pending\":true")));
    assert!(
        lines
            .iter()
            .all(|l| l.contains("\"recorded_for\":\"main\""))
    );
}

#[test]
fn recording_on_another_branch_writes_no_pending_lines() {
    let repo = Repo::new();
    let r = repo.record("feat/x", cut_session(), RecordRedaction::Off);
    assert_eq!(r.pending_written, 0);
    assert!(repo.lines().iter().all(|l| !l.contains("pending")));
}

#[test]
fn recording_on_the_cut_branch_resolves_its_pre_cut_main_records() {
    let repo = Repo::new();
    repo.record("main", cut_session(), RecordRedaction::Off);
    let r = repo.record("feat/x", cut_session(), RecordRedaction::Off);
    let stats = r.resolution.expect("a non-default branch resolves");
    assert_eq!(stats.resolved, 2);
    assert_eq!(stats.uncertain(), 0);
    assert_eq!(r.resolutions_appended, 2);
    let resolved: Vec<_> = repo
        .lines()
        .into_iter()
        .filter(|l| l.contains("\"resolved\""))
        .collect();
    assert_eq!(resolved.len(), 2);
    for l in &resolved {
        assert!(l.contains("\"recorded_for\":\"feat/x\""), "{l}");
        assert!(l.contains("\"from\":\"main\""), "{l}");
        assert!(l.contains("\"basis\":\"timeline-forward\""), "{l}");
        assert!(!l.contains("\"pending\""), "{l}");
    }
    // The branch's report now holds all three calls, and nothing is left pending.
    let report = repo.report("feat/x", None);
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.pending.lines, 0);
    // Resolving again appends nothing.
    let again = repo.record("feat/x", cut_session(), RecordRedaction::Off);
    assert_eq!(again.resolutions_appended, 0);
}

#[test]
fn an_ambiguous_session_stays_pending_and_is_counted() {
    let repo = Repo::new();
    // Two work branches at one instant: the timeline cannot place the main call before them.
    let session = vec![
        rec("s1", "req-a", "main", Some(1)),
        rec("s1", "req-c", "feat/x", Some(3)),
        rec("s1", "req-d", "feat/y", Some(3)),
    ];
    repo.record("main", session.clone(), RecordRedaction::Off);
    let r = repo.record("feat/x", session, RecordRedaction::Off);
    let stats = r.resolution.unwrap();
    assert_eq!(stats.resolved, 0);
    assert_eq!(stats.ambiguous, 1);
    assert_eq!(r.resolutions_appended, 0);
    assert_eq!(repo.report("feat/x", None).summary.pending.lines, 1);
}

#[test]
fn a_call_after_returning_to_main_stays_pending_and_is_never_carried_back() {
    let repo = Repo::new();
    let session = vec![
        rec("s1", "req-a", "main", Some(1)),
        rec("s1", "req-c", "feat/x", Some(3)),
        rec("s1", "req-e", "main", Some(5)),
    ];
    repo.record("main", session.clone(), RecordRedaction::Off);
    let r = repo.record("feat/x", session, RecordRedaction::Off);
    let stats = r.resolution.unwrap();
    assert_eq!(stats.resolved, 1);
    assert_eq!(stats.carried_back, 1);
    assert_eq!(repo.report("feat/x", None).summary.pending.lines, 1);
}

#[test]
fn a_redacted_ledger_resolves_under_the_matching_salt() {
    let repo = Repo::new();
    let keyed = RecordRedaction::Keyed { names: false };
    repo.record("main", cut_session(), keyed);
    let r = repo.record("feat/x", cut_session(), keyed);
    let stats = r.resolution.unwrap();
    assert_eq!(stats.resolved, 2, "{stats:?}");
    assert_eq!(stats.no_evidence, 0);
    let text = repo.lines().join("\n");
    assert!(!text.contains("req-a"), "ids stay hashed");
    assert_eq!(
        repo.lines()
            .iter()
            .filter(|l| l.contains("\"resolved\""))
            .count(),
        2
    );
}

/// A source whose transcripts for the repository are gone (no roots discovered).
struct NoTranscripts;

impl Source for NoTranscripts {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }
    fn discover(
        &self,
        _projects_root: &Path,
        _repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError> {
        Ok(Vec::new())
    }
    fn read(
        &self,
        _roots: &[TranscriptRoot],
        _sink: &mut dyn FnMut(Record),
    ) -> Result<ReadStats, SourceError> {
        Ok(ReadStats::default())
    }
    fn dispatches(&self, _roots: &[TranscriptRoot]) -> Result<Vec<Dispatch>, SourceError> {
        Ok(Vec::new())
    }
}

#[test]
fn resolve_at_on_the_default_branch_is_a_no_op_without_reading_transcripts() {
    // Nothing is resolved on the default branch, so it needs only git: a repository with no
    // transcript directory still gets the no-op outcome, not a missing-transcripts error.
    let repo = Repo::new();
    let out = resolve_at(
        &repo.git(),
        &NoTranscripts,
        Path::new("/projects"),
        &repo.root(),
        Some("main"),
        &LedgerTarget::Default,
        &FixedClock,
        false,
    )
    .expect("the default branch needs no transcripts");
    assert!(out.on_default_branch);
    assert_eq!(out.stats.resolved, 0);
    assert_eq!(out.appended.appended, 0);
}

#[test]
fn resolve_at_and_the_live_report_report_skipped_ledger_lines() {
    // A malformed line might have been a pending one: its count is carried out, not dropped.
    let repo = Repo::new();
    repo.record("main", cut_session()[..2].to_vec(), RecordRedaction::Off);
    let mut ledger = std::fs::OpenOptions::new()
        .append(true)
        .open(repo.ledger())
        .unwrap();
    std::io::Write::write_all(&mut ledger, b"not json\n").unwrap();
    let out = repo.resolve("feat/x", cut_session(), true);
    assert_eq!(out.ledger_stats.malformed, 1);
    let live = run(
        &repo.git(),
        &FakeSource(cut_session()),
        Path::new("/projects"),
        &repo.root(),
        Some("feat/x"),
        &Options::new(Mode::Strict),
        PriceTable::bundled(),
    )
    .unwrap();
    assert_eq!(
        live.pending_ledger_stats.map(|s| s.malformed),
        Some(1),
        "the live report must carry the default ledger's skipped lines"
    );
}

#[test]
fn resolve_at_dry_run_counts_and_writes_nothing() {
    let repo = Repo::new();
    repo.record("main", cut_session()[..2].to_vec(), RecordRedaction::Off);
    let before = std::fs::read(repo.ledger()).unwrap();
    let out = repo.resolve("feat/x", cut_session(), true);
    assert!(out.dry_run);
    assert_eq!(out.stats.resolved, 2);
    assert_eq!(out.appended.appended, 0);
    assert_eq!(std::fs::read(repo.ledger()).unwrap(), before);
    // For real, then again: idempotent.
    let out = repo.resolve("feat/x", cut_session(), false);
    assert_eq!(out.appended.appended, 2);
    assert_eq!(
        repo.resolve("feat/x", cut_session(), false)
            .appended
            .appended,
        0
    );
}

#[test]
fn resolve_at_only_resolves_for_the_named_branch() {
    let repo = Repo::new();
    repo.record("main", cut_session()[..2].to_vec(), RecordRedaction::Off);
    let out = repo.resolve("feat/y", cut_session(), false);
    assert_eq!(out.stats.resolved, 0);
    assert_eq!(out.stats.other_branch, 2);
    assert_eq!(out.appended.appended, 0);
    assert_eq!(repo.lines().len(), 2);
}

#[test]
fn the_report_counts_unresolved_pending_lines_in_markdown_and_json() {
    let repo = Repo::new();
    // No timestamps: the lines can be recorded but never placed.
    let undated = vec![
        rec("s1", "req-a", "main", None),
        rec("s1", "req-c", "feat/x", Some(3)),
    ];
    repo.record("main", undated.clone(), RecordRedaction::Off);
    let r = repo.record("feat/x", undated.clone(), RecordRedaction::Off);
    assert_eq!(r.resolution.unwrap().no_timestamp, 1);
    let report = repo.report("feat/x", None);
    assert_eq!(report.summary.pending.lines, 1);
    assert_eq!(report.summary.pending.calls, 1);
    let md = markdown(&report.summary, &report.costs, true);
    assert!(md.contains("pending ledger line"), "{md}");
    let js = json(&report.summary, &report.costs, true);
    assert!(js.contains("\"pending\": { \"lines\": 1"), "{js}");
}

#[test]
fn the_live_report_reads_the_default_ledger_for_pending_lines() {
    let repo = Repo::new();
    let undated = vec![
        rec("s1", "req-a", "main", None),
        rec("s1", "req-c", "feat/x", Some(3)),
    ];
    repo.record("main", undated.clone(), RecordRedaction::Off);
    let live = run(
        &repo.git(),
        &FakeSource(undated),
        Path::new("/projects"),
        &repo.root(),
        Some("feat/x"),
        // Strict: claim mode would count the session's undated pre-cut call, so it would not be pending.
        &Options::new(Mode::Strict),
        PriceTable::bundled(),
    )
    .unwrap();
    assert_eq!(live.summary.pending.lines, 1);
}

#[test]
fn the_live_report_has_no_pending_without_a_tally_dir() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let git = FakeGit::new().with_toplevel(&root, &root).with_worktrees(
        &root,
        vec![Worktree {
            path: root.clone(),
            branch: "main".to_owned(),
        }],
    );
    let live = run(
        &git,
        &FakeSource(cut_session()),
        Path::new("/projects"),
        &root,
        Some("feat/x"),
        &Options::default(),
        PriceTable::bundled(),
    )
    .unwrap();
    assert_eq!(live.summary.pending.lines, 0);
    assert!(live.pending_unavailable.is_none());
}

#[cfg(unix)]
#[test]
fn an_unreadable_default_ledger_is_reported_not_read_as_zero_pending() {
    // A refused ledger (a symlink) is not an empty one: the live report still builds, but says
    // its pending usage is unknown rather than claiming there is none (Governance II).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    std::fs::create_dir(root.join(".tally")).unwrap();
    let victim = root.join("victim");
    std::fs::write(&victim, b"").unwrap();
    std::os::unix::fs::symlink(&victim, root.join(".tally").join("ledger.jsonl")).unwrap();
    let git = FakeGit::new().with_toplevel(&root, &root).with_worktrees(
        &root,
        vec![Worktree {
            path: root.clone(),
            branch: "main".to_owned(),
        }],
    );
    let live = run(
        &git,
        &FakeSource(cut_session()),
        Path::new("/projects"),
        &root,
        Some("feat/x"),
        &Options::default(),
        PriceTable::bundled(),
    )
    .unwrap();
    assert_eq!(live.summary.pending.lines, 0);
    assert!(
        live.pending_unavailable.is_some(),
        "a refused ledger must be reported, not read as zero pending"
    );
}

fn capture(session: &str, branch: &str, second: u32) -> CaptureEvent {
    CaptureEvent {
        v: CAPTURE_SCHEMA_VERSION,
        harness: Harness::ClaudeCode,
        kind: CaptureKind::Stop,
        session_id: SessionId::new(session),
        agent_id: None,
        branch: branch.to_owned(),
        worktree_branch: None,
        ts: format!("2026-01-01T00:00:{second:02}Z"),
    }
}

/// A session that never stamps a work branch: only a capture can place its main calls.
fn uncut_session() -> Vec<Record> {
    vec![
        rec("s1", "req-a", "main", Some(1)),
        rec("s1", "req-b", "main", Some(2)),
    ]
}

#[test]
fn a_capture_resolves_a_session_the_transcript_cannot_place() {
    let repo = Repo::new();
    repo.record("main", uncut_session(), RecordRedaction::Off);
    append_captures(&repo.root(), &[capture("s1", "feat/x", 4)]).unwrap();
    let r = repo.record("feat/x", uncut_session(), RecordRedaction::Off);
    assert_eq!(r.resolution.unwrap().resolved, 2);
    let lines = repo.lines();
    let resolved: Vec<_> = lines
        .iter()
        .filter(|l| l.contains("\"resolved\""))
        .collect();
    assert_eq!(resolved.len(), 2);
    assert!(
        resolved
            .iter()
            .all(|l| l.contains("\"basis\":\"captured\""))
    );
}

#[test]
fn an_earlier_capture_on_the_default_branch_does_not_hide_the_work_branch_capture() {
    // Decision 0006: the evidence is the session's first capture *with a work branch* at or after
    // the call. A capture still on `main` (second 3) comes first but places nothing.
    let repo = Repo::new();
    repo.record("main", uncut_session(), RecordRedaction::Off);
    append_captures(
        &repo.root(),
        &[capture("s1", "main", 3), capture("s1", "feat/x", 4)],
    )
    .unwrap();
    let r = repo.record("feat/x", uncut_session(), RecordRedaction::Off);
    let stats = r.resolution.unwrap();
    assert_eq!((stats.resolved, stats.no_work_branch), (2, 0));
    let lines = repo.lines();
    assert!(
        lines
            .iter()
            .filter(|l| l.contains("\"resolved\""))
            .all(|l| l.contains("\"basis\":\"captured\""))
    );
}

#[test]
fn without_a_capture_the_same_session_stays_pending_and_is_counted() {
    let repo = Repo::new();
    repo.record("main", uncut_session(), RecordRedaction::Off);
    let r = repo.record("feat/x", uncut_session(), RecordRedaction::Off);
    let stats = r.resolution.unwrap();
    assert_eq!((stats.resolved, stats.no_work_branch), (0, 2));
}

#[test]
fn a_capture_that_disagrees_with_the_transcript_leaves_the_line_pending() {
    let repo = Repo::new();
    repo.record("main", cut_session(), RecordRedaction::Off);
    append_captures(&repo.root(), &[capture("s1", "feat/y", 4)]).unwrap();
    let r = repo.record("feat/x", cut_session(), RecordRedaction::Off);
    let stats = r.resolution.unwrap();
    assert_eq!((stats.resolved, stats.disagreement), (0, 2));
}

#[test]
fn a_claim_mode_record_on_the_branch_covers_the_pending_lines_without_a_second_line() {
    let repo = Repo::new();
    repo.record("main", cut_session(), RecordRedaction::Off);
    // Claim mode records the whole session for feat/x itself, so nothing is left to resolve.
    let r = repo.record_in(Mode::Claim, "feat/x", cut_session(), RecordRedaction::Off);
    let stats = r.resolution.unwrap();
    assert_eq!((stats.resolved, stats.covered), (0, 2));
    assert_eq!(r.resolutions_appended, 0);
    assert_eq!(repo.report("feat/x", None).summary.pending.lines, 0);
}

// Decision 0006, Reporting: a pending line is in no branch's totals (the default branch's
// included) and appears only in `Summary.pending`; a request the report already counts is left
// out of `pending`.

/// Rewrites the ledger line for `request` so it is an ordinary (not pending) line.
fn unpend(repo: &Repo, request: &str) {
    let needle = format!("\"{request}\"");
    let text: Vec<String> = repo
        .lines()
        .into_iter()
        .map(|l| {
            if l.contains(&needle) {
                l.replace(",\"pending\":true", "")
            } else {
                l
            }
        })
        .collect();
    std::fs::write(repo.ledger(), text.join("\n") + "\n").unwrap();
}

fn main_records() -> Vec<Record> {
    vec![
        rec("s1", "req-a", "main", Some(1)),
        rec("s1", "req-b", "main", Some(2)),
        rec("s1", "req-m", "main", Some(3)),
    ]
}

fn live_run(repo: &Repo, mode: Mode, records: Vec<Record>) -> branch_tally_core::report::Report {
    run(
        &repo.git(),
        &FakeSource(records),
        Path::new("/projects"),
        &repo.root(),
        Some("feat/x"),
        &Options::new(mode),
        PriceTable::bundled(),
    )
    .unwrap()
}

fn ledger_with_live(
    repo: &Repo,
    branch: &str,
    mode: Mode,
    live: Vec<Record>,
) -> branch_tally_core::report::Report {
    run_from_ledger_at(
        &repo.git(),
        Some(&FakeSource(live) as &dyn Source),
        Path::new("/projects"),
        &repo.root(),
        Some(branch),
        &Options::new(mode),
        &LedgerTarget::Default,
        PriceTable::bundled(),
    )
    .unwrap()
}

#[test]
fn a_default_branch_ledger_report_leaves_pending_lines_out_of_its_totals() {
    let repo = Repo::new();
    repo.record("main", main_records(), RecordRedaction::Off);
    unpend(&repo, "req-m");
    let report = repo.report("main", None);
    assert_eq!(report.summary.total.calls, 1, "only the ordinary line");
    assert_eq!(report.summary.pending.calls, 2);
    assert_eq!(report.summary.pending.tokens, 30);
}

#[test]
fn a_default_branch_report_with_live_counts_a_live_call_once_and_not_as_pending() {
    let repo = Repo::new();
    repo.record("main", main_records(), RecordRedaction::Off);
    unpend(&repo, "req-m");
    let report = ledger_with_live(&repo, "main", Mode::Strict, main_records());
    // Live strict selection counts all three for main, so none is also pending.
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.pending.lines, 0);
}

#[test]
fn a_live_report_does_not_count_a_pre_cut_call_it_attributes_forward_as_pending() {
    let repo = Repo::new();
    repo.record("main", cut_session(), RecordRedaction::Off);
    let report = live_run(&repo, Mode::Timeline, cut_session());
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.pending.lines, 0, "counted in the total");
}

#[test]
fn a_live_report_leaves_a_call_it_does_not_count_in_pending() {
    let repo = Repo::new();
    repo.record("main", cut_session(), RecordRedaction::Off);
    // Strict mode counts only the call stamped feat/x; the two pre-cut calls stay pending.
    let report = live_run(&repo, Mode::Strict, cut_session());
    assert_eq!(report.summary.total.calls, 1);
    assert_eq!(report.summary.pending.lines, 2);
}

#[test]
fn a_redacted_live_report_does_not_count_a_pre_cut_call_it_attributes_forward_as_pending() {
    let repo = Repo::new();
    let keyed = RecordRedaction::Keyed { names: false };
    repo.record("main", cut_session(), keyed);
    let report = live_run(&repo, Mode::Timeline, cut_session());
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.pending.lines, 0, "hashed ids matched");
}

#[test]
fn a_ledger_report_with_live_does_not_count_a_forward_attributed_call_as_pending() {
    let repo = Repo::new();
    repo.record("main", cut_session()[..2].to_vec(), RecordRedaction::Off);
    repo.record(
        "feat/x",
        vec![cut_session()[2].clone()],
        RecordRedaction::Off,
    );
    // The ledger holds only req-c for feat/x; live timeline attributes both pre-cut calls forward.
    let report = ledger_with_live(&repo, "feat/x", Mode::Timeline, cut_session());
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.pending.lines, 0);
}

#[test]
fn a_redacted_ledger_report_with_live_does_not_count_a_forward_attributed_call_as_pending() {
    let repo = Repo::new();
    let keyed = RecordRedaction::Keyed { names: false };
    repo.record("main", cut_session()[..2].to_vec(), keyed);
    repo.record("feat/x", vec![cut_session()[2].clone()], keyed);
    let report = ledger_with_live(&repo, "feat/x", Mode::Timeline, cut_session());
    assert_eq!(report.summary.total.calls, 3);
    assert_eq!(report.summary.pending.lines, 0);
}

#[test]
fn a_default_branch_ledger_of_only_pending_lines_reports_zero_totals_and_the_pending() {
    let repo = Repo::new();
    repo.record("main", main_records(), RecordRedaction::Off);
    let report = repo.report("main", None);
    assert_eq!(report.summary.total.calls, 0);
    assert_eq!(report.summary.pending.lines, 3);
    assert_eq!(report.summary.pending.tokens, 45);
    let md = markdown(&report.summary, &report.costs, true);
    assert!(md.contains("pending ledger line"), "{md}");
    let js = json(&report.summary, &report.costs, true);
    assert!(js.contains("\"pending\": { \"lines\": 3"), "{js}");
}

#[test]
fn a_live_report_with_no_counted_calls_but_pending_lines_still_reports_them() {
    let repo = Repo::new();
    repo.record("main", cut_session(), RecordRedaction::Off);
    // Live transcripts hold only calls for another branch: nothing counted, two pending.
    let other = vec![rec("s9", "req-z", "feat/z", Some(5))];
    let report = live_run(&repo, Mode::Strict, other);
    assert_eq!(report.summary.total.calls, 0);
    assert_eq!(report.summary.pending.lines, 2);
}

#[test]
fn no_records_and_no_pending_is_still_no_records() {
    let repo = Repo::new();
    let err = run(
        &repo.git(),
        &FakeSource(vec![rec("s9", "req-z", "feat/z", Some(5))]),
        Path::new("/projects"),
        &repo.root(),
        Some("feat/x"),
        &Options::new(Mode::Strict),
        PriceTable::bundled(),
    );
    assert!(matches!(
        err,
        Err(branch_tally_core::report::ReportError::NoRecords { .. })
    ));
}
