//! Integration tests for `report`, built from in-code records and fakes (no transcript content).

#![expect(clippy::panic, reason = "a wrong error variant fails the test loudly")]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::model::{
    AgentId, Dispatch, Harness, Mode, Origin, PriceTableSource, Record, RequestId, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::redact::RedactOptions;
use branch_tally_core::report::{Options, ReportError, build, run};
use branch_tally_core::sources::{
    ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot,
};

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
    /// Whether discovery finds a transcript directory. A source with records has one.
    has_root: bool,
    repo_seen: RefCell<Option<PathBuf>>,
}

impl FakeSource {
    fn new(records: Vec<Record>, stats: ReadStats) -> Self {
        Self {
            records,
            stats,
            has_root: true,
            repo_seen: RefCell::new(None),
        }
    }

    /// A source whose discovery finds no transcript directory for the repository.
    fn without_roots() -> Self {
        Self {
            has_root: false,
            ..Self::new(Vec::new(), ReadStats::default())
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
        Ok(if self.has_root {
            vec![TranscriptRoot::new("/projects/-repo", RootKind::Repository)]
        } else {
            Vec::new()
        })
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
    fn dispatches(&self, _roots: &[TranscriptRoot]) -> Result<Vec<Dispatch>, SourceError> {
        Ok(Vec::new())
    }
}

fn bundled() -> &'static PriceTable {
    PriceTable::bundled()
}

/// A main-checkout session with a skill, plus a worktree subagent dispatched from it.
fn redaction_input() -> (Vec<Record>, Vec<Dispatch>) {
    let mut main = rec("sess-main-secret", "a", Some("feat/x"));
    main.attribution.skill = Some("skill-secret".to_owned());
    main.ts = "2026-01-02T03:04:05Z".parse().ok();
    let mut sub = rec("sess-sub-secret", "b", None);
    sub.agent_id = Some(AgentId::new("agentid-secret"));
    sub.is_sidechain = true;
    sub.parent_session = Some(SessionId::new("sess-main-secret"));
    sub.attribution.agent = Some("agent-secret".to_owned());
    sub.cwd = Some("/repo/.claude/worktrees/wt-secret".to_owned());
    sub.origin = Some(Origin::Worktree("wt-secret".to_owned()));
    sub.ts = "2026-01-02T03:05:05Z".parse().ok();
    let dispatch = Dispatch {
        session_id: SessionId::new("sess-main-secret"),
        agent_id: AgentId::new("agentid-secret"),
        git_branch: Some("feat/x".to_owned()),
        ts: None,
    };
    (vec![main, sub], vec![dispatch])
}

fn build_with(options: &Options<'_>) -> branch_tally_core::report::Report {
    let (records, dispatches) = redaction_input();
    match build(
        records,
        &dispatches,
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        options,
        bundled(),
    ) {
        Ok(report) => report,
        Err(e) => panic!("fixture must build: {e}"),
    }
}

const SECRETS: [&str; 5] = [
    "sess-main-secret",
    "sess-sub-secret",
    "agentid-secret",
    "wt-secret",
    "2026-01-02T03",
];

#[test]
fn build_without_redaction_is_not_redacted_and_keeps_names() {
    let report = build_with(&Options::default());
    assert!(!report.summary.redacted);
    let dump = format!("{:?}", report.summary);
    assert!(dump.contains("sess-main-secret"));
    assert!(dump.contains("wt-secret"));
}

#[test]
fn build_with_redaction_hides_ids_and_names_but_not_costs() {
    let plain = build_with(&Options::default());
    assert!(
        !plain.summary.subagents.rows.is_empty(),
        "fixture must yield subagent rows"
    );
    let report = build_with(&Options::default().with_redaction(RedactOptions::default()));
    assert!(report.summary.redacted);
    let dump = format!("{:?}", report.summary);
    for secret in SECRETS.iter().chain(&["skill-secret", "agent-secret"]) {
        assert!(!dump.contains(secret), "{secret} leaked");
    }
    assert_eq!(report.costs, plain.costs);
    assert_eq!(report.summary.total, plain.summary.total);
    assert_eq!(
        report.summary.subagents.rows.len(),
        plain.summary.subagents.rows.len()
    );
}

#[test]
fn build_with_redaction_keeping_names_hides_ids_only() {
    let plain = build_with(&Options::default());
    let report = build_with(&Options::default().with_redaction(RedactOptions { names: false }));
    assert!(report.summary.redacted);
    let dump = format!("{:?}", report.summary);
    for secret in SECRETS {
        assert!(!dump.contains(secret), "{secret} leaked");
    }
    assert!(dump.contains("skill-secret"));
    assert!(dump.contains("agent-secret"));
    assert_eq!(report.costs, plain.costs);
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
    let report = build(
        records,
        &[],
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &Options::default(),
        bundled(),
    )
    .unwrap();
    assert_eq!(report.branch, "feat/x");
    assert_eq!(report.summary.mode, Mode::Claim);
    assert_eq!(report.summary.total.calls, 2);
    assert_eq!(report.read_stats.malformed_lines, 3);
    assert!(report.worktrees_unavailable.is_none());
}

#[test]
fn build_reports_strict_mode() {
    let records = vec![rec("s1", "a", Some("feat/x"))];
    let report = build(
        records,
        &[],
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &Options::new(Mode::Strict),
        bundled(),
    )
    .unwrap();
    assert_eq!(report.summary.mode, Mode::Strict);
}

#[test]
fn build_with_nothing_selected_is_an_explicit_outcome_with_stats() {
    let records = vec![rec("s1", "a", Some("other"))];
    let err = build(
        records,
        &[],
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &Options::default(),
        bundled(),
    )
    .unwrap_err();
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
        &Options::default(),
        bundled(),
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
        &Options::default(),
        bundled(),
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
        &Options::default(),
        bundled(),
    )
    .unwrap_err();
    assert!(matches!(err, ReportError::Git(_)), "{err}");
}

#[test]
fn run_with_no_transcript_directory_names_the_repository_and_every_slug_tried() {
    // #40: empty discovery is its own outcome, not "no records on this branch". `D:\x` has two
    // candidate slugs (drive form and Git Bash form); the error lists both, lowercased.
    let git = FakeGit::new().with_main_checkout(r"D:\x\wt", r"D:\x");
    let source = FakeSource::without_roots();
    let err = run(
        &git,
        &source,
        Path::new("/projects"),
        Path::new(r"D:\x\wt"),
        Some("feat/x"),
        &Options::default(),
        bundled(),
    )
    .unwrap_err();
    match &err {
        ReportError::NoTranscriptDirectory { repo, slugs } => {
            assert_eq!(repo, Path::new(r"D:\x"));
            assert_eq!(slugs, &["d--x".to_owned(), "-d-x".to_owned()]);
        }
        other => panic!("expected NoTranscriptDirectory, got {other}"),
    }
    assert_eq!(
        err.to_string(),
        r"No transcript directory for D:\x (expected one of the slugs d--x, -d-x)"
    );
}

#[test]
fn no_transcript_directory_with_one_slug_keeps_the_reference_message() {
    let err = ReportError::NoTranscriptDirectory {
        repo: PathBuf::from("/home/u/x"),
        slugs: vec!["-home-u-x".to_owned()],
    };
    assert_eq!(
        err.to_string(),
        "No transcript directory for /home/u/x (expected slug -home-u-x)"
    );
}

const CUSTOM: &str =
    "as_of = \"2031-02-03\"\n\n[models.\"m\"]\ninput = 1000000.0\noutput = 2000000.0\n";

#[expect(
    clippy::unwrap_used,
    reason = "a fixture that fails to parse should fail the test loudly"
)]
fn custom() -> PriceTable {
    PriceTable::parse(CUSTOM, PriceTableSource::Override).unwrap()
}

#[test]
fn build_prices_with_the_table_the_caller_supplies() {
    let records = vec![rec("s1", "a", Some("feat/x"))];
    let table = custom();
    let report = build(
        records,
        &[],
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &Options::default(),
        &table,
    )
    .unwrap();
    // 10 input at $1M/MTok = $10, 5 output at $2M/MTok = $10.
    assert_eq!(report.costs.cost_for("m"), Some(20.0));
    assert_eq!(report.costs.price_table.as_of, "2031-02-03");
    assert_eq!(report.costs.price_table.source, PriceTableSource::Override);
}

#[test]
fn run_prices_with_the_table_the_caller_supplies() {
    let git = FakeGit::new();
    let source = FakeSource::new(vec![rec("s1", "a", Some("feat/x"))], stats());
    let table = custom();
    let report = run(
        &git,
        &source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some("feat/x"),
        &Options::default(),
        &table,
    )
    .unwrap();
    assert_eq!(report.costs.cost_for("m"), Some(20.0));
    assert_eq!(report.costs.price_table.as_of, "2031-02-03");
    assert_eq!(report.costs.price_table.source, PriceTableSource::Override);
}

// --- #6: subagent rollup, end to end on the scrubbed fixtures -------------------------------

use branch_tally_core::model::Workspace;
use branch_tally_core::report::Report;
use branch_tally_core::sources::claude_code::ClaudeCode;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("claude-code")
        .join(name)
}

/// Runs the real Claude Code source over a fixture with a fake git whose live worktrees are
/// `worktrees`, as `(directory name, branch)` under `C:\repo\.claude\worktrees`.
fn fixture_report(
    name: &str,
    branch: &str,
    mode: Mode,
    worktrees: &[(&str, &str)],
) -> Result<Report, ReportError> {
    let repo = PathBuf::from(r"C:\repo");
    let mut list = vec![Worktree {
        path: repo.clone(),
        branch: "main".to_owned(),
    }];
    for (dir, b) in worktrees {
        list.push(Worktree {
            path: repo.join(".claude").join("worktrees").join(dir),
            branch: (*b).to_owned(),
        });
    }
    let git = FakeGit::new().with_worktrees(&repo, list);
    run(
        &git,
        &ClaudeCode,
        &fixture(name),
        &repo,
        Some(branch),
        &Options::new(mode),
        bundled(),
    )
}

#[test]
fn unstamped_main_checkout_calls_are_not_put_on_the_checkouts_current_branch() {
    // #37: the main checkout is on `main` in the fake worktree list. Its unstamped calls
    // (`req_2`, `req_5`) must not land there just because that is what is checked out now.
    let main = fixture_report("unstamped-main", "main", Mode::Claim, &[]).unwrap();
    assert_eq!((main.summary.total.calls, main.summary.total.input), (1, 4));
    // `s1`'s only work branch is `feat/x`, so its unstamped call is still claimed there.
    let feat = fixture_report("unstamped-main", "feat/x", Mode::Claim, &[]).unwrap();
    assert_eq!((feat.summary.total.calls, feat.summary.total.input), (3, 6));
}

#[test]
fn unstamped_calls_are_counted_repository_wide_whichever_branch_is_reported() {
    // #60: `req_2` is claimed onto `feat/x` (session `s1`'s only work branch), so only `req_5`
    // (1,115 tokens) is placed by no branch. The count is the same on either branch.
    for branch in ["main", "feat/x"] {
        let r = fixture_report("unstamped-main", branch, Mode::Claim, &[]).unwrap();
        assert_eq!(
            (r.summary.unstamped.calls, r.summary.unstamped.tokens),
            (1, 1_115),
            "{branch}"
        );
        let json = branch_tally_core::render::json(&r.summary, &r.costs);
        assert!(json.contains("\"unstamped\""), "{branch}: {json}");
    }
    // Strict has no claim, so both `(none)` calls (the reference's bucket) are unplaced.
    let strict = fixture_report("unstamped-main", "main", Mode::Strict, &[]).unwrap();
    assert_eq!(
        (
            strict.summary.unstamped.calls,
            strict.summary.unstamped.tokens
        ),
        (2, 2_221)
    );
    // A fixture with every call stamped reports none, and omits the key.
    let none = fixture_report("subagent", "feat/x", Mode::Claim, &[]).unwrap();
    assert_eq!(none.summary.unstamped.calls, 0);
    assert!(!branch_tally_core::render::json(&none.summary, &none.costs).contains("unstamped"));
}

#[test]
fn removed_worktrees_roll_up_to_the_dispatcher_branch() {
    let r = fixture_report("subagent-removed-worktree", "feat/x", Mode::Claim, &[]).unwrap();
    assert_eq!(r.summary.total.calls, 7);
    assert_eq!(r.summary.total.input, 28);
    let sub = &r.summary.subagents;
    assert_eq!(sub.calls(), 5);
    assert_eq!(sub.dispatched(), 3);
    assert!(sub.rows.iter().all(|x| x.workspace.is_worktree()));
    assert_eq!((sub.unattributed_calls, sub.unattributed_tokens), (0, 0));
    let json = branch_tally_core::render::json(&r.summary, &r.costs);
    assert!(json.contains("\"subagents\""), "{json}");
}

#[test]
fn a_renamed_worktree_branch_does_not_steal_the_subagent_calls() {
    let live = [("agent-e1", "chore/x-part")];
    let r = fixture_report("subagent-renamed-branch", "feat/x", Mode::Claim, &live).unwrap();
    assert_eq!(r.summary.total.calls, 4);
    assert_eq!(r.summary.total.input, 10);
    let rows = &r.summary.subagents.rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subagent, "e1");
    assert_eq!(rows[0].workspace, Workspace::worktree("agent-e1"));
}

#[test]
fn strict_mode_stays_raw_and_has_no_subagent_breakdown() {
    let live = [("agent-e1", "chore/x-part")];
    let r = fixture_report(
        "subagent-renamed-branch",
        "chore/x-part",
        Mode::Strict,
        &live,
    )
    .unwrap();
    assert_eq!(r.summary.total.calls, 1);
    assert_eq!(r.summary.total.input, 4);
    assert!(r.summary.subagents.is_empty());
}

#[test]
fn a_subagent_with_no_dispatch_link_is_counted_unattributed_not_guessed() {
    let r = fixture_report("subagent-no-dispatch-link", "feat/x", Mode::Claim, &[]).unwrap();
    // Decision 6: an unlinked subagent record still takes part in the session claim like any
    // stamped record, so the total holds all four calls (1 + 2 + 5 + 6 input). The two unlinked
    // calls are also counted as unattributed.
    assert_eq!(r.summary.total.calls, 4);
    assert_eq!(r.summary.total.input, 14);
    // Requirement 7: the breakdown covers only rolled-up records (n1's one call). n2's two
    // unlinked calls are in the total via the claim and reported as such, not as rows.
    assert_eq!(r.summary.subagents.calls(), 1);
    assert_eq!(r.summary.subagents.unattributed_calls, 2);
    assert!(r.summary.subagents.unattributed_tokens > 0);
    assert_eq!(r.summary.subagents.unlinked_in_total_calls, 2);
    assert_eq!(
        r.summary.subagents.unlinked_in_total_tokens,
        r.summary.subagents.unattributed_tokens
    );
}

#[test]
fn unusable_dispatchers_keep_raw_stamps_and_are_counted() {
    let r = fixture_report("subagent-dispatcher-unusable", "feat/x", Mode::Claim, &[]).unwrap();
    // All five calls (1..=5 input) ride the session claims; g2 and g3 stay counted unattributed.
    assert_eq!(r.summary.total.input, 15);
    assert_eq!(r.summary.subagents.unattributed_calls, 2);
    assert_eq!(r.summary.subagents.unlinked_in_total_calls, 2);
}

/// A wrapper that forwards everything, as the CLI's `CheckedSource` must. `Source::dispatches`
/// has no default, so leaving it out here would not compile.
struct Forwarding(ClaudeCode);

impl Source for Forwarding {
    fn harness(&self) -> Harness {
        self.0.harness()
    }
    fn discover(
        &self,
        projects_root: &Path,
        repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError> {
        self.0.discover(projects_root, repo)
    }
    fn read(
        &self,
        roots: &[TranscriptRoot],
        sink: &mut dyn FnMut(Record),
    ) -> Result<ReadStats, SourceError> {
        self.0.read(roots, sink)
    }
    fn dispatches(&self, roots: &[TranscriptRoot]) -> Result<Vec<Dispatch>, SourceError> {
        self.0.dispatches(roots)
    }
}

#[test]
fn a_forwarding_wrapper_source_still_links_subagents() {
    let repo = PathBuf::from(r"C:\repo");
    let git = FakeGit::new().with_worktrees(
        &repo,
        vec![Worktree {
            path: repo.clone(),
            branch: "main".to_owned(),
        }],
    );
    let r = run(
        &git,
        &Forwarding(ClaudeCode),
        &fixture("subagent-removed-worktree"),
        &repo,
        Some("feat/x"),
        &Options::default(),
        bundled(),
    )
    .unwrap();
    assert_eq!(r.summary.subagents.unattributed_calls, 0);
    assert_eq!(r.summary.subagents.calls(), 5);
}

#[test]
fn a_main_checkout_subagent_rolls_up_in_the_main_workspace() {
    let r = fixture_report("subagent-main-checkout", "feat/x", Mode::Claim, &[]).unwrap();
    assert_eq!(r.summary.total.calls, 3);
    assert_eq!(r.summary.total.input, 6);
    let rows = &r.summary.subagents.rows;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].workspace.is_worktree());
}

#[test]
fn subagents_sharing_a_worktree_get_one_row_each() {
    let r = fixture_report("subagent-shared-worktree", "feat/x", Mode::Claim, &[]).unwrap();
    assert_eq!(r.summary.total.input, 10);
    let rows = &r.summary.subagents.rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].subagent, "explorer");
    assert_eq!(rows[0].calls, 2);
    assert_eq!(rows[1].subagent, "h2");
}

// --- #7: timeline mode, the work-branch rule and the default-branch fallback ----------------

use branch_tally_core::model::{DefaultBranchSource, WorkBranchRule};
use branch_tally_core::report::{DefaultBranch, WorkBranch};

fn rec_at(session: &str, request: &str, branch: &str, ts: &str) -> Record {
    let mut r = rec(session, request, Some(branch));
    r.ts = ts.parse().ok();
    r
}

/// Feature work, a stretch stamped `main`, then feature work again, in one session.
fn timeline_records() -> Vec<Record> {
    vec![
        rec_at("s1", "a", "feat/x", "2026-01-01T00:00:01Z"),
        rec_at("s1", "b", "main", "2026-01-01T00:00:02Z"),
        rec_at("s1", "c", "feat/x", "2026-01-01T00:00:03Z"),
    ]
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper that fails to report should fail the test loudly"
)]
fn timeline_build(branch: &str, opts: &Options<'_>) -> Report {
    build(
        timeline_records(),
        &[],
        ReadStats::default(),
        Path::new("/repo"),
        &[],
        branch,
        opts,
        bundled(),
    )
    .unwrap()
}

#[test]
fn timeline_mode_carries_a_default_branch_stamp_back_to_the_work_branch() {
    let opts = Options::new(Mode::Timeline).with_default_branch(DefaultBranch {
        name: "main".to_owned(),
        source: DefaultBranchSource::OriginHead,
    });
    let r = timeline_build("feat/x", &opts);
    assert_eq!(r.summary.mode, Mode::Timeline);
    assert_eq!(r.summary.total.calls, 3);
    assert_eq!(r.summary.footnote.heuristics.timeline_carried.calls, 1);
    assert_eq!(
        r.summary.work_branch,
        Some(WorkBranchRule::ExcludesDefault {
            branch: "main".to_owned(),
            source: DefaultBranchSource::OriginHead,
        })
    );
}

#[test]
fn timeline_default_rule_excludes_the_resolved_default_not_a_hardcoded_main() {
    // With "trunk" as the default, `main` is an ordinary work branch: its record stays there.
    let opts = Options::new(Mode::Timeline).with_default_branch(DefaultBranch {
        name: "trunk".to_owned(),
        source: DefaultBranchSource::OriginHead,
    });
    assert_eq!(timeline_build("feat/x", &opts).summary.total.calls, 2);
}

#[test]
fn timeline_without_a_resolved_default_assumes_main_and_says_so() {
    let r = timeline_build("feat/x", &Options::new(Mode::Timeline));
    assert_eq!(r.summary.total.calls, 3);
    assert_eq!(
        r.summary.work_branch,
        Some(WorkBranchRule::ExcludesDefault {
            branch: "main".to_owned(),
            source: DefaultBranchSource::AssumedMain,
        })
    );
}

#[test]
fn timeline_custom_rule_is_used_and_its_pattern_reported() {
    // `main` is a work branch under this rule, so its record stays on it.
    let any = |b: &str| b == "feat/x" || b == "main";
    let opts = Options::new(Mode::Timeline).with_work_branch(WorkBranch::Custom {
        pattern: "^(feat/x|main)$",
        matches: &any,
    });
    let r = timeline_build("feat/x", &opts);
    assert_eq!(r.summary.total.calls, 2);
    assert_eq!(
        r.summary.work_branch,
        Some(WorkBranchRule::Custom {
            pattern: "^(feat/x|main)$".to_owned()
        })
    );
}

#[test]
fn claim_and_strict_record_no_work_branch_rule() {
    for mode in [Mode::Claim, Mode::Strict] {
        let r = timeline_build("feat/x", &Options::new(mode));
        assert_eq!(r.summary.work_branch, None, "{mode:?}");
    }
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper that fails to report should fail the test loudly"
)]
fn run_timeline(git: &FakeGit) -> Report {
    let source = FakeSource::new(timeline_records(), ReadStats::default());
    run(
        git,
        &source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some("feat/x"),
        &Options::new(Mode::Timeline),
        bundled(),
    )
    .unwrap()
}

fn rule_of(r: &Report) -> (String, DefaultBranchSource) {
    match &r.summary.work_branch {
        Some(WorkBranchRule::ExcludesDefault { branch, source }) => (branch.clone(), *source),
        other => panic!("expected the default rule, got {other:?}"),
    }
}

#[test]
fn run_resolves_the_default_branch_from_origin_head_first() {
    let git = FakeGit::new()
        .with_default_branch("/repo", "trunk")
        .with_init_default_branch("/repo", "develop");
    assert_eq!(
        rule_of(&run_timeline(&git)),
        ("trunk".to_owned(), DefaultBranchSource::OriginHead)
    );
}

#[test]
fn run_falls_back_to_init_default_branch_and_says_so() {
    let git = FakeGit::new().with_init_default_branch("/repo", "develop");
    assert_eq!(
        rule_of(&run_timeline(&git)),
        ("develop".to_owned(), DefaultBranchSource::InitDefaultBranch)
    );
}

#[test]
fn run_assumes_main_when_neither_is_available_and_says_so() {
    assert_eq!(
        rule_of(&run_timeline(&FakeGit::new())),
        ("main".to_owned(), DefaultBranchSource::AssumedMain)
    );
}

#[test]
fn run_with_a_custom_rule_reports_the_pattern_and_ignores_git_defaults() {
    let any = |_: &str| true;
    let opts = Options::new(Mode::Timeline).with_work_branch(WorkBranch::Custom {
        pattern: ".",
        matches: &any,
    });
    let source = FakeSource::new(timeline_records(), ReadStats::default());
    let r = run(
        &FakeGit::new().with_default_branch("/repo", "trunk"),
        &source,
        Path::new("/projects"),
        Path::new("/repo"),
        Some("feat/x"),
        &opts,
        bundled(),
    )
    .unwrap();
    assert_eq!(
        r.summary.work_branch,
        Some(WorkBranchRule::Custom {
            pattern: ".".to_owned()
        })
    );
}

#[test]
fn timeline_ambiguous_unstamped_call_in_an_unselected_session_is_still_counted() {
    // #60 audit: an unstamped call with no parseable timestamp is timeline-ambiguous and keeps
    // `(none)`. Its session has no record on `feat/x`, so the session-scoped ambiguous count
    // never sees it; `unstamped` must.
    let mut stray = rec("s2", "z", None);
    stray.ts = None;
    let records = vec![
        rec_at("s1", "a", "feat/x", "2026-01-01T00:00:01Z"),
        rec_at("s2", "y", "feat/y", "2026-01-01T00:00:01Z"),
        stray,
    ];
    let opts = Options::new(Mode::Timeline).with_default_branch(DefaultBranch {
        name: "main".to_owned(),
        source: DefaultBranchSource::OriginHead,
    });
    let r = build(
        records,
        &[],
        ReadStats::default(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &opts,
        bundled(),
    )
    .unwrap();
    assert_eq!(r.summary.footnote.heuristics.ambiguous.calls, 0);
    assert_eq!(
        (r.summary.unstamped.calls, r.summary.unstamped.tokens),
        (1, 15)
    );
}

/// Unstamped records whose token sum saturates must mark the report's totals unreliable (#96).
#[test]
fn saturated_unstamped_count_marks_totals_overflowed() {
    let mut huge = rec("s-none", "huge", None);
    huge.usage.input = u64::MAX;
    let mut huge2 = rec("s-none2", "huge2", None);
    huge2.usage.input = u64::MAX;
    let records = vec![rec("s1", "a", Some("feat/x")), huge, huge2];
    let report = build(
        records,
        &[],
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &Options::default(),
        bundled(),
    )
    .unwrap();
    assert_eq!(report.summary.unstamped.tokens, u64::MAX);
    assert!(report.summary.footnote.totals_overflowed);
}

#[test]
fn unsaturated_unstamped_count_does_not_flag_overflow() {
    let records = vec![rec("s1", "a", Some("feat/x")), rec("s-none", "n", None)];
    let report = build(
        records,
        &[],
        stats(),
        Path::new("/repo"),
        &[],
        "feat/x",
        &Options::default(),
        bundled(),
    )
    .unwrap();
    assert_eq!(report.summary.unstamped.calls, 1);
    assert!(!report.summary.footnote.totals_overflowed);
}
