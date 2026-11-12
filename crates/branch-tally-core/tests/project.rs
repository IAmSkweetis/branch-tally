//! Integration tests for the project-wide aggregate (#87): synthetic tempdir ledgers, no
//! transcript content.

#![expect(clippy::unwrap_used, reason = "tests fail loudly on a wrong result")]

use std::path::{Path, PathBuf};

use branch_tally_core::git::{FakeGit, Worktree};
use branch_tally_core::ledger::{self, Clock, Identity, Ledger};
use branch_tally_core::model::{
    AttributedRecord, BranchBasis, Harness, Mode, PriceTableSource, ProjectReport, Record,
    RequestId, SelectedRecord, Selection, SessionId, Usage,
};
use branch_tally_core::pricing::PriceTable;
use branch_tally_core::report::{Options, run_from_ledger};

const TABLE: &str = "as_of = \"2026-01-31\"\n[models.\"priced\"]\ninput = 1\noutput = 2\n";

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> String {
        "2026-01-01T00:00:00Z".to_owned()
    }
}

fn table() -> PriceTable {
    PriceTable::parse(TABLE, PriceTableSource::Override).unwrap()
}

/// One call of 1M input and 1M output tokens: $3.00 on the `priced` model.
fn sel(id: &str, model: &str, ts: Option<&str>, basis: BranchBasis) -> SelectedRecord {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new(format!("s-{id}")),
        RequestId::new(id),
        Usage {
            input: 1_000_000,
            output: 1_000_000,
            ..Usage::default()
        },
    );
    r.model = Some(model.to_owned());
    r.ts = ts.map(str::to_owned);
    SelectedRecord {
        attributed: AttributedRecord {
            record: r,
            branch: "x".to_owned(),
            basis,
            subagent: None,
        },
        kept_by: Selection::Exact,
    }
}

fn plain(id: &str, model: &str, ts: Option<&str>) -> SelectedRecord {
    sel(id, model, ts, BranchBasis::Stamped)
}

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".tally").join("ledger.jsonl");
        Self { _dir: dir, path }
    }

    fn record(&self, branch: &str, records: &[SelectedRecord]) {
        ledger::append(
            &self.path,
            records,
            branch,
            Mode::Claim,
            Some(Path::new("/repo")),
            &Identity,
            &FixedClock,
        )
        .unwrap();
    }

    fn read(&self) -> Ledger {
        ledger::read(&self.path).unwrap()
    }
}

fn build(f: &Fixture) -> ProjectReport {
    branch_tally_core::project::build(&f.read(), Some(Path::new("/repo")), Mode::Claim, &table())
}

#[test]
fn groups_rows_by_recorded_for_and_dates_each_by_its_latest_ts() {
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[
            plain("a1", "priced", Some("2026-01-02T00:00:00Z")),
            plain("a2", "priced", Some("2026-01-05T00:00:00Z")),
        ],
    );
    f.record(
        "feat/b",
        &[plain("b1", "priced", Some("2026-01-03T00:00:00Z"))],
    );
    let report = build(&f);
    assert_eq!(report.rows.len(), 2);
    let a = report.rows.iter().find(|r| r.branch == "feat/a").unwrap();
    assert_eq!(a.usage.calls, 2);
    assert_eq!(a.last_call.as_deref(), Some("2026-01-05T00:00:00Z"));
    assert!((a.cost.known - 6.0).abs() < 1e-9);
    assert!(!a.is_floor());
}

#[test]
fn shared_request_counts_once_in_totals_and_is_reported_as_overlap() {
    let f = Fixture::new();
    let shared = plain("shared", "priced", Some("2026-01-02T00:00:00Z"));
    f.record(
        "feat/a",
        &[
            shared.clone(),
            plain("a1", "priced", Some("2026-01-02T00:00:00Z")),
        ],
    );
    f.record("feat/b", &[shared]);
    let report = build(&f);
    let row_calls: u64 = report.rows.iter().map(|r| r.usage.calls).sum();
    assert_eq!(row_calls, 3);
    assert_eq!(report.totals.usage.calls, 2);
    assert!((report.totals.cost.known - 6.0).abs() < 1e-9);
    assert_eq!(report.totals.overlap.calls, 1);
    assert_eq!(report.totals.overlap.tokens, 2_000_000);
    assert!((report.totals.overlap.cost.known - 3.0).abs() < 1e-9);
}

#[test]
fn no_overlap_when_nothing_is_shared() {
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[plain("a1", "priced", Some("2026-01-02T00:00:00Z"))],
    );
    f.record(
        "feat/b",
        &[plain("b1", "priced", Some("2026-01-03T00:00:00Z"))],
    );
    let report = build(&f);
    assert_eq!(report.totals.overlap.calls, 0);
    assert_eq!(report.totals.usage.calls, 2);
}

#[test]
fn unpriced_model_makes_row_totals_and_later_series_points_floors() {
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[plain("a1", "priced", Some("2026-01-01T00:00:00Z"))],
    );
    f.record(
        "feat/b",
        &[plain("b1", "mystery", Some("2026-01-02T00:00:00Z"))],
    );
    f.record(
        "feat/c",
        &[plain("c1", "priced", Some("2026-01-03T00:00:00Z"))],
    );
    let report = build(&f);
    let b = report.rows.iter().find(|r| r.branch == "feat/b").unwrap();
    assert!(b.is_floor());
    assert_eq!(b.unpriced_models, vec!["mystery".to_owned()]);
    assert!(report.totals.is_floor());
    assert_eq!(report.totals.unpriced_models, vec!["mystery".to_owned()]);
    assert!(report.is_floor());
    let floors: Vec<bool> = report.series.iter().map(|p| p.is_floor).collect();
    assert_eq!(floors, vec![false, true, true]);
}

#[test]
fn undated_branch_is_counted_and_kept_off_the_series() {
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[plain("a1", "priced", Some("2026-01-01T00:00:00Z"))],
    );
    f.record("feat/none", &[plain("n1", "priced", None)]);
    let report = build(&f);
    assert_eq!(report.undated_branches, 1);
    assert_eq!(report.rows.len(), 2);
    let none = report
        .rows
        .iter()
        .find(|r| r.branch == "feat/none")
        .unwrap();
    assert_eq!(none.last_call, None);
    assert_eq!(report.series.len(), 1);
    assert_eq!(report.totals.usage.calls, 2);
}

#[test]
fn series_is_ordered_by_date_and_cumulative_without_double_counting_shared_calls() {
    let f = Fixture::new();
    let shared = plain("shared", "priced", Some("2026-01-01T00:00:00Z"));
    // Recorded newest first, to prove ordering is by date and not by ledger order.
    f.record(
        "feat/late",
        &[
            shared.clone(),
            plain("l1", "priced", Some("2026-03-01T00:00:00Z")),
        ],
    );
    f.record("feat/early", &[shared]);
    f.record(
        "feat/mid",
        &[plain("m1", "priced", Some("2026-02-01T00:00:00Z"))],
    );
    let report = build(&f);
    let order: Vec<&str> = report.series.iter().map(|p| p.branch.as_str()).collect();
    assert_eq!(order, vec!["feat/early", "feat/mid", "feat/late"]);
    let dates: Vec<&str> = report.series.iter().map(|p| p.date.as_str()).collect();
    assert!(dates.windows(2).all(|w| w[0] <= w[1]));
    let cum: Vec<f64> = report.series.iter().map(|p| p.cumulative_cost).collect();
    assert!((cum[0] - 3.0).abs() < 1e-9);
    assert!((cum[1] - 6.0).abs() < 1e-9);
    assert!((cum[2] - 9.0).abs() < 1e-9);
    assert!((report.totals.cost.known - cum[2]).abs() < 1e-9);
    let row_order: Vec<&str> = report.rows.iter().map(|r| r.branch.as_str()).collect();
    assert_eq!(row_order, order);
}

#[test]
fn uncertain_calls_are_carried_to_rows_and_totals() {
    let f = Fixture::new();
    let amb = sel(
        "amb",
        "priced",
        Some("2026-01-01T00:00:00Z"),
        BranchBasis::TimelineAmbiguous,
    );
    f.record("feat/a", std::slice::from_ref(&amb));
    f.record("feat/b", &[amb]);
    let report = build(&f);
    for row in &report.rows {
        assert_eq!(row.uncertain.ambiguous.calls, 1);
    }
    assert_eq!(report.totals.uncertain.ambiguous.calls, 1);
    assert_eq!(report.totals.uncertain.ambiguous.tokens, 2_000_000);
}

#[test]
fn empty_ledger_gives_an_empty_report() {
    let f = Fixture::new();
    let report = build(&f);
    assert_eq!(report.rows.len(), 0);
    assert_eq!(report.series.len(), 0);
    assert_eq!(report.totals.usage.calls, 0);
    assert_eq!(report.undated_branches, 0);
}

#[test]
fn each_row_matches_the_single_branch_ledger_report() {
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[
            plain("a1", "priced", Some("2026-01-01T00:00:00Z")),
            plain("a2", "mystery", Some("2026-01-02T00:00:00Z")),
            sel(
                "a3",
                "priced",
                Some("2026-01-02T00:00:00Z"),
                BranchBasis::TimelineAmbiguous,
            ),
        ],
    );
    f.record(
        "feat/b",
        &[plain("b1", "priced", Some("2026-01-03T00:00:00Z"))],
    );
    let report = build(&f);
    let git = FakeGit::new().with_worktrees(
        "/repo",
        vec![Worktree {
            path: PathBuf::from("/repo"),
            branch: "main".to_owned(),
        }],
    );
    assert_eq!(report.rows.len(), 2);
    for row in &report.rows {
        let single = run_from_ledger(
            &git,
            None,
            Path::new("/projects"),
            Path::new("/repo"),
            Some(&row.branch),
            &Options::default(),
            &f.path,
            &table(),
        )
        .unwrap();
        assert_eq!(row.usage, single.summary.total, "{}", row.branch);
        assert_eq!(row.uncertain, single.summary.footnote.heuristics);
        assert!((row.cost.known - single.costs.total).abs() < 1e-12);
        assert_eq!(row.cost.unpriced_models, single.costs.unpriced_models);
        assert_eq!(row.is_floor(), single.costs.is_floor());
    }
}

#[test]
fn last_call_is_the_latest_instant_across_mixed_offsets() {
    let f = Fixture::new();
    // 00:30+01:00 is 23:30Z the day before: earlier than 00:00Z, though later as text.
    f.record(
        "feat/a",
        &[
            plain("a1", "priced", Some("2026-01-02T00:30:00+01:00")),
            plain("a2", "priced", Some("2026-01-02T00:00:00Z")),
        ],
    );
    let report = build(&f);
    assert_eq!(
        report.rows[0].last_call.as_deref(),
        Some("2026-01-02T00:00:00Z")
    );
}

#[test]
fn mixed_precision_and_offsets_order_the_series_by_instant() {
    let f = Fixture::new();
    // As text ".500Z" sorts before "Z" and "+01:00" after; by instant the order is the offset
    // one (23:30Z), the whole second (00:00:00Z), then the fractional one (00:00:00.5Z).
    f.record(
        "feat/frac",
        &[plain("f1", "priced", Some("2026-01-02T00:00:00.500Z"))],
    );
    f.record(
        "feat/whole",
        &[plain("w1", "priced", Some("2026-01-02T00:00:00Z"))],
    );
    f.record(
        "feat/offset",
        &[plain("o1", "priced", Some("2026-01-02T00:30:00+01:00"))],
    );
    let report = build(&f);
    let rows: Vec<&str> = report.rows.iter().map(|r| r.branch.as_str()).collect();
    assert_eq!(rows, vec!["feat/offset", "feat/whole", "feat/frac"]);
    let series: Vec<&str> = report.series.iter().map(|p| p.branch.as_str()).collect();
    assert_eq!(series, rows);
}

#[test]
fn unparsable_ts_never_wins_and_an_all_unparsable_branch_is_undated() {
    let f = Fixture::new();
    // "9999..." sorts after every real date as text; it must not be picked as latest.
    f.record(
        "feat/mixed",
        &[
            plain("m1", "priced", Some("2026-01-02T00:00:00Z")),
            plain("m2", "priced", Some("9999-not-a-time")),
        ],
    );
    f.record(
        "feat/junk",
        &[plain("j1", "priced", Some("9999-not-a-time"))],
    );
    let report = build(&f);
    let mixed = report
        .rows
        .iter()
        .find(|r| r.branch == "feat/mixed")
        .unwrap();
    assert_eq!(mixed.last_call.as_deref(), Some("2026-01-02T00:00:00Z"));
    assert_eq!(mixed.usage.calls, 2, "the unparsable call is still counted");
    let junk = report
        .rows
        .iter()
        .find(|r| r.branch == "feat/junk")
        .unwrap();
    assert_eq!(junk.last_call, None);
    assert_eq!(report.undated_branches, 1);
    assert_eq!(report.series.len(), 1);
    assert_eq!(report.rows.last().unwrap().branch, "feat/junk");
    assert_eq!(report.totals.usage.calls, 3);
}

#[test]
fn overlap_on_an_unpriced_model_is_a_floor_and_names_the_model() {
    let f = Fixture::new();
    let shared = plain("shared", "mystery", Some("2026-01-02T00:00:00Z"));
    f.record("feat/a", std::slice::from_ref(&shared));
    f.record("feat/b", &[shared]);
    let report = build(&f);
    assert_eq!(report.totals.overlap.calls, 1);
    assert!(report.totals.overlap.cost.is_floor());
    assert!(report.totals.is_floor());
    assert_eq!(report.totals.unpriced_models, vec!["mystery".to_owned()]);
}

/// The work-branch rule only drives live timeline attribution. A ledger-only read never applies
/// it, so a custom rule changes neither `run_from_ledger` nor the project row, and `build` needs
/// no rule parameter.
#[test]
fn a_custom_work_branch_rule_does_not_change_ledger_only_numbers() {
    use branch_tally_core::report::WorkBranch;
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[
            plain("a1", "priced", Some("2026-01-01T00:00:00Z")),
            sel(
                "a2",
                "priced",
                Some("2026-01-02T00:00:00Z"),
                BranchBasis::TimelineAmbiguous,
            ),
        ],
    );
    let git = FakeGit::new().with_worktrees(
        "/repo",
        vec![Worktree {
            path: PathBuf::from("/repo"),
            branch: "main".to_owned(),
        }],
    );
    let nothing = |_: &str| false;
    let default_rule = Options::new(Mode::Timeline);
    let custom_rule = Options::new(Mode::Timeline).with_work_branch(WorkBranch::Custom {
        pattern: "^never$",
        matches: &nothing,
    });
    let run = |options: &Options<'_>| {
        run_from_ledger(
            &git,
            None,
            Path::new("/projects"),
            Path::new("/repo"),
            Some("feat/a"),
            options,
            &f.path,
            &table(),
        )
        .unwrap()
    };
    let (with_default, with_custom) = (run(&default_rule), run(&custom_rule));
    assert_eq!(with_default.summary, with_custom.summary);
    assert_eq!(with_default.costs, with_custom.costs);
    assert_eq!(with_custom.summary.work_branch, None);

    let report = branch_tally_core::project::build(
        &f.read(),
        Some(Path::new("/repo")),
        Mode::Timeline,
        &table(),
    );
    let row = &report.rows[0];
    assert_eq!(row.usage, with_custom.summary.total);
    assert_eq!(row.uncertain, with_custom.summary.footnote.heuristics);
    assert!((row.cost.known - with_custom.costs.total).abs() < 1e-12);
}

#[test]
fn calls_without_a_usable_ts_are_counted_per_branch_and_project_wide() {
    let f = Fixture::new();
    f.record(
        "feat/a",
        &[
            plain("a1", "priced", Some("2026-01-05T00:00:00Z")),
            plain("a2", "priced", None),
            plain("a3", "priced", Some("garbage")),
        ],
    );
    f.record(
        "feat/b",
        &[plain("b1", "priced", Some("2026-01-06T00:00:00Z"))],
    );
    f.record("feat/c", &[plain("c1", "priced", None)]);
    let report = build(&f);
    let count = |name: &str| {
        report
            .rows
            .iter()
            .find(|r| r.branch == name)
            .unwrap()
            .undated_calls
    };
    assert_eq!(count("feat/a"), 2);
    assert_eq!(count("feat/b"), 0);
    assert_eq!(count("feat/c"), 1);
    assert_eq!(report.totals.undated_calls, 3);
}

#[test]
fn a_request_shared_by_two_branches_with_no_ts_is_counted_once_project_wide() {
    let f = Fixture::new();
    let shared = plain("s", "priced", None);
    f.record("feat/a", std::slice::from_ref(&shared));
    f.record("feat/b", &[shared]);
    let report = build(&f);
    assert_eq!(report.rows[0].undated_calls, 1);
    assert_eq!(report.rows[1].undated_calls, 1);
    assert_eq!(report.totals.undated_calls, 1);
}
