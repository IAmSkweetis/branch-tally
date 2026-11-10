//! Integration tests for `report`, built from in-code records and fakes (no transcript content).

#![expect(clippy::panic, reason = "a wrong error variant fails the test loudly")]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::model::{
    Dispatch, Harness, Mode, Origin, PriceTableSource, Record, RequestId, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
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
    fn dispatches(&self, _roots: &[TranscriptRoot]) -> Result<Vec<Dispatch>, SourceError> {
        Ok(Vec::new())
    }
}

fn bundled() -> &'static PriceTable {
    PriceTable::bundled()
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
        false,
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
        true,
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
        false,
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
        false,
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
        false,
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
        false,
        bundled(),
    )
    .unwrap_err();
    assert!(matches!(err, ReportError::Git(_)), "{err}");
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
        false,
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
        false,
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
    strict: bool,
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
        strict,
        bundled(),
    )
}

#[test]
fn removed_worktrees_roll_up_to_the_dispatcher_branch() {
    let r = fixture_report("subagent-removed-worktree", "feat/x", false, &[]).unwrap();
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
    let r = fixture_report("subagent-renamed-branch", "feat/x", false, &live).unwrap();
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
    let r = fixture_report("subagent-renamed-branch", "chore/x-part", true, &live).unwrap();
    assert_eq!(r.summary.total.calls, 1);
    assert_eq!(r.summary.total.input, 4);
    assert!(r.summary.subagents.is_empty());
}

#[test]
fn a_subagent_with_no_dispatch_link_is_counted_unattributed_not_guessed() {
    let r = fixture_report("subagent-no-dispatch-link", "feat/x", false, &[]).unwrap();
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
    let r = fixture_report("subagent-dispatcher-unusable", "feat/x", false, &[]).unwrap();
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
        false,
        bundled(),
    )
    .unwrap();
    assert_eq!(r.summary.subagents.unattributed_calls, 0);
    assert_eq!(r.summary.subagents.calls(), 5);
}

#[test]
fn a_main_checkout_subagent_rolls_up_in_the_main_workspace() {
    let r = fixture_report("subagent-main-checkout", "feat/x", false, &[]).unwrap();
    assert_eq!(r.summary.total.calls, 3);
    assert_eq!(r.summary.total.input, 6);
    let rows = &r.summary.subagents.rows;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].workspace.is_worktree());
}

#[test]
fn subagents_sharing_a_worktree_get_one_row_each() {
    let r = fixture_report("subagent-shared-worktree", "feat/x", false, &[]).unwrap();
    assert_eq!(r.summary.total.input, 10);
    let rows = &r.summary.subagents.rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].subagent, "explorer");
    assert_eq!(rows[0].calls, 2);
    assert_eq!(rows[1].subagent, "h2");
}
