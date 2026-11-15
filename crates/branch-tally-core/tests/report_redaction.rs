//! `record` and `run_from_ledger_at` against a redacted or mixed ledger (#67, decision 0005):
//! live calls match redacted lines in hashed form and are never counted twice. Synthetic ids only.

#![expect(clippy::unwrap_used, reason = "tests fail loudly on a wrong result")]

use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::ledger::{Clock, Identity, KeyedRedact, LedgerTarget, Salt, TallyDir};
use branch_tally_core::model::{
    AgentId, Dispatch, Harness, LiveMatchFailure, Origin, Record, RequestId, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::report::{LiveRedactionMatch, Options, Report, record, run_from_ledger_at};
use branch_tally_core::sources::{
    ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot,
};

struct FakeSource(Vec<Record>, Vec<Dispatch>);

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
        Ok(self.1.clone())
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> String {
        "2026-01-01T00:00:00Z".to_owned()
    }
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
    fn git(&self) -> FakeGit {
        FakeGit::new().with_worktrees(
            self.root(),
            vec![Worktree {
                path: self.root(),
                branch: "main".to_owned(),
            }],
        )
    }
    fn salt(&self) -> Salt {
        let tally = TallyDir::open(&self.root()).unwrap().unwrap();
        Salt::load_or_create(&tally).unwrap()
    }
    fn record(&self, source: &FakeSource, redact: &dyn branch_tally_core::ledger::Redact) -> u64 {
        self.record_on("feat/x", source, redact)
    }
    fn record_on(
        &self,
        branch: &str,
        source: &FakeSource,
        redact: &dyn branch_tally_core::ledger::Redact,
    ) -> u64 {
        record(
            &self.git(),
            source,
            Path::new("/projects"),
            &self.root(),
            Some(branch),
            &Options::default(),
            &self.ledger(),
            redact,
            &FixedClock,
        )
        .unwrap()
        .outcome
        .appended
    }
    fn report(&self, source: Option<&dyn Source>) -> Report {
        self.report_on("feat/x", source)
    }
    fn report_on(&self, branch: &str, source: Option<&dyn Source>) -> Report {
        let ledger = self.ledger();
        run_from_ledger_at(
            &self.git(),
            source,
            Path::new("/projects"),
            &self.root(),
            Some(branch),
            &Options::default(),
            &LedgerTarget::Explicit(&ledger),
            PriceTable::bundled(),
        )
        .unwrap()
    }
}

fn rec(session: &str, request: &str, output: u64) -> Record {
    rec_on("feat/x", session, request, output)
}

fn rec_on(branch: &str, session: &str, request: &str, output: u64) -> Record {
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

/// Two main-checkout calls and one made by a subagent in a worktree.
fn source(repo: &Repo) -> FakeSource {
    let mut sub = rec("sub-session", "req-sub", 4);
    sub.agent_id = Some(AgentId::new("agentabc"));
    sub.parent_session = Some(SessionId::new("s1"));
    sub.is_sidechain = true;
    sub.git_branch = None;
    sub.cwd = Some(
        repo.root()
            .join(".claude")
            .join("worktrees")
            .join("agent-abc")
            .to_string_lossy()
            .into_owned(),
    );
    sub.origin = Some(Origin::worktree("agent-abc"));
    FakeSource(
        vec![rec("s1", "req-a", 1), rec("s1", "req-b", 2), sub],
        vec![Dispatch {
            session_id: SessionId::new("s1"),
            agent_id: AgentId::new("agentabc"),
            git_branch: Some("feat/x".to_owned()),
            ts: None,
        }],
    )
}

fn calls(r: &Report) -> String {
    format!("{:?}|{}", r.summary.total, r.costs.total)
}

#[test]
fn recording_twice_under_keyed_redaction_appends_nothing_the_second_time() {
    let repo = Repo::new();
    let src = source(&repo);
    let red = KeyedRedact::new(repo.salt(), false);
    assert_eq!(repo.record(&src, &red), 3);
    assert_eq!(repo.record(&src, &red), 0);
    assert_eq!(
        std::fs::read_to_string(repo.ledger())
            .unwrap()
            .lines()
            .count(),
        3
    );
}

#[test]
fn a_redacted_ledger_plus_the_same_live_calls_counts_each_once() {
    let repo = Repo::new();
    let src = source(&repo);
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    let alone = repo.report(None);
    let both = repo.report(Some(&src));
    assert_eq!(
        calls(&both),
        calls(&alone),
        "live calls were double counted"
    );
    let info = both.ledger.unwrap();
    assert_eq!((info.live_only, info.in_both), (0, 3));
    assert_eq!(info.live_redaction, LiveRedactionMatch::Matched);
}

#[test]
fn a_mixed_ledger_plus_live_counts_each_distinct_call_once() {
    let repo = Repo::new();
    let src = source(&repo);
    // req-a unredacted, then all three redacted: req-a is the documented twice-recorded case.
    let first = FakeSource(vec![src.0[0].clone()], src.1.clone());
    repo.record(&first, &Identity);
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    let ledger_alone = repo.report(None);
    let both = repo.report(Some(&src));
    assert_eq!(calls(&both), calls(&ledger_alone));
    let stats = both.ledger.unwrap().ledger_stats;
    assert!(stats.is_mixed());
    assert_eq!((stats.unredacted_lines, stats.redacted_lines), (1, 3));
}

#[test]
fn a_live_only_call_is_added_once_to_a_redacted_ledger() {
    let repo = Repo::new();
    let src = source(&repo);
    let two = FakeSource(src.0[..2].to_vec(), src.1.clone());
    repo.record(&two, &KeyedRedact::new(repo.salt(), false));
    let info = repo.report(Some(&src)).ledger.unwrap();
    assert_eq!((info.live_only, info.in_both), (1, 2));
}

#[test]
fn a_missing_salt_is_surfaced_and_not_guessed() {
    let repo = Repo::new();
    let src = source(&repo);
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    std::fs::remove_file(repo.root().join(".tally").join("salt")).unwrap();
    let info = repo.report(Some(&src)).ledger.unwrap();
    assert_eq!(info.live_redaction, LiveRedactionMatch::NoSalt);
    assert_eq!(info.in_both, 0, "nothing can be matched without the salt");
}

#[test]
fn a_salt_that_does_not_match_the_lines_is_surfaced() {
    let repo = Repo::new();
    let src = source(&repo);
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    let other = Salt::from_bytes([7; 32]);
    std::fs::write(
        repo.root().join(".tally").join("salt"),
        format!("{}\n", other.to_hex()),
    )
    .unwrap();
    let info = repo.report(Some(&src)).ledger.unwrap();
    assert_eq!(info.live_redaction, LiveRedactionMatch::FingerprintMismatch);
}

#[test]
fn an_unredacted_ledger_needs_no_salt() {
    let repo = Repo::new();
    let src = source(&repo);
    repo.record(&src, &Identity);
    let info = repo.report(Some(&src)).ledger.unwrap();
    assert_eq!(info.live_redaction, LiveRedactionMatch::NotNeeded);
    assert_eq!(info.in_both, 3);
}

#[test]
fn an_ambiguous_call_is_not_counted_twice_in_the_repository_wide_counts() {
    let repo = Repo::new();
    // A subagent with its own branch stamp and no dispatch: placed, but not with certainty.
    let mut unlinked = rec("sub-2", "req-unlinked", 8);
    unlinked.agent_id = Some(AgentId::new("agentxyz"));
    unlinked.parent_session = Some(SessionId::new("s1"));
    unlinked.is_sidechain = true;
    let src = FakeSource(vec![rec("s1", "req-a", 1), unlinked], Vec::new());
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    let alone = repo.report(None);
    let both = repo.report(Some(&src));
    let ambiguous = |r: &Report| r.summary.footnote.heuristics.ambiguous.calls;
    assert_eq!(
        ambiguous(&alone),
        1,
        "the fixture must exercise the heuristic: {:?}",
        alone.summary.footnote
    );
    assert_eq!(ambiguous(&both), 1);
}

#[test]
fn the_footnote_flags_a_mixed_ledger_with_its_counts() {
    let repo = Repo::new();
    let src = source(&repo);
    let first = FakeSource(vec![src.0[0].clone()], src.1.clone());
    repo.record(&first, &Identity);
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    let risk = repo
        .report(Some(&src))
        .summary
        .footnote
        .double_count
        .unwrap();
    let mixed = risk.mixed_ledger.unwrap();
    assert_eq!(
        (
            mixed.unredacted_lines,
            mixed.redacted_lines,
            mixed.distinct_salts
        ),
        (1, 3, 1)
    );
    assert_eq!(risk.live_match, None);
    // A ledger-only report says so too.
    assert!(repo.report(None).summary.footnote.double_count.is_some());
}

#[test]
fn the_footnote_flags_a_live_match_failure_and_is_silent_when_clean() {
    let repo = Repo::new();
    let src = source(&repo);
    repo.record(&src, &KeyedRedact::new(repo.salt(), false));
    assert_eq!(
        repo.report(Some(&src)).summary.footnote.double_count,
        None,
        "a matched redacted ledger is not a risk"
    );
    std::fs::remove_file(repo.root().join(".tally").join("salt")).unwrap();
    let risk = repo
        .report(Some(&src))
        .summary
        .footnote
        .double_count
        .unwrap();
    assert_eq!(risk.live_match, Some(LiveMatchFailure::NoSalt));
    assert_eq!(risk.mixed_ledger, None);

    let plain = Repo::new();
    plain.record(&source(&plain), &Identity);
    assert_eq!(
        plain
            .report(Some(&source(&plain)))
            .summary
            .footnote
            .double_count,
        None
    );
}

#[test]
fn the_salt_is_read_from_the_tally_dir_that_holds_the_ledger() {
    // A linked worktree keeps its own ledger and salt; the main checkout has neither.
    let main = tempfile::tempdir().unwrap();
    let wt = Repo::new();
    let git = FakeGit::new()
        .with_toplevel(wt.root(), wt.root())
        .with_main_checkout(wt.root(), main.path())
        .with_worktrees(
            main.path(),
            vec![Worktree {
                path: main.path().to_owned(),
                branch: "main".to_owned(),
            }],
        );
    let src = source(&wt);
    let red = KeyedRedact::new(wt.salt(), false);
    branch_tally_core::report::record_at(
        &git,
        &src,
        Path::new("/projects"),
        &wt.root(),
        Some("feat/x"),
        &Options::default(),
        &LedgerTarget::Default,
        &red,
        &FixedClock,
    )
    .unwrap();
    let run = |source: Option<&dyn Source>| {
        run_from_ledger_at(
            &git,
            source,
            Path::new("/projects"),
            &wt.root(),
            Some("feat/x"),
            &Options::default(),
            &LedgerTarget::Default,
            PriceTable::bundled(),
        )
        .unwrap()
    };
    let both = run(Some(&src));
    let info = both.ledger.as_ref().unwrap();
    assert_eq!(info.live_redaction, LiveRedactionMatch::Matched);
    assert_eq!((info.live_only, info.in_both), (0, 3));
    assert_eq!(calls(&both), calls(&run(None)));
    assert!(
        !main.path().join(".tally").exists(),
        "nothing is created in the main checkout"
    );
}

#[test]
fn an_agent_branch_finds_its_redacted_and_unredacted_lines() {
    let repo = Repo::new();
    let branch = "worktree-agent-abc123";
    let src = FakeSource(
        vec![
            rec_on(branch, "s1", "req-a", 1),
            rec_on(branch, "s1", "req-b", 2),
        ],
        Vec::new(),
    );
    repo.record_on(branch, &src, &KeyedRedact::new(repo.salt(), false));
    let redacted_only = repo.report_on(branch, None);
    assert_eq!(redacted_only.summary.total.calls, 2);
    assert_eq!(
        redacted_only.summary.footnote.off_branch_tokens, 0,
        "hashed branch names must not read as off-branch"
    );
    // One call also recorded raw: the same agent branch in both spellings, counted and noted.
    let raw = FakeSource(vec![src.0[0].clone()], Vec::new());
    repo.record_on(branch, &raw, &Identity);
    let mixed = repo.report_on(branch, None);
    assert_eq!(mixed.summary.total.calls, 3);
    assert!(mixed.summary.footnote.double_count.is_some());
    let both = repo.report_on(branch, Some(&src));
    assert_eq!(both.summary.total.calls, 3);
    assert_eq!(both.summary.footnote.off_branch_tokens, 0);
}

#[test]
fn one_hashed_call_under_both_branch_spellings_is_counted_once() {
    let repo = Repo::new();
    let branch = "worktree-agent-abc123";
    let src = FakeSource(
        vec![
            rec_on(branch, "s1", "req-a", 1),
            rec_on(branch, "s1", "req-b", 2),
        ],
        Vec::new(),
    );
    repo.record_on(branch, &src, &KeyedRedact::new(repo.salt(), false));
    // Respell every line's branch as the raw name, as a ledger written before branch hashing
    // would hold it: the same hashed request ids now sit under both spellings.
    let text = std::fs::read_to_string(repo.ledger()).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let hashed_branch = first["recorded_for"].as_str().unwrap().to_owned();
    assert_ne!(hashed_branch, branch, "the fixture must hash the branch");
    let respelled = text.replace(&hashed_branch, branch);
    std::fs::write(repo.ledger(), format!("{text}{respelled}")).unwrap();
    let report = repo.report_on(branch, None);
    assert_eq!(report.summary.total.calls, 2, "a call was counted twice");
    assert_eq!(report.ledger.unwrap().collapsed_lines, 2);
}
