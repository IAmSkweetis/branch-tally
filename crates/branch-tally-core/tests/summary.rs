//! Integration tests for `summary`, built from in-code records (no transcript content).

use branch_tally_core::model::{AgentId, SubagentSummary, SubagentTags, Workspace};
use branch_tally_core::model::{
    AttributedRecord, BranchBasis, Harness, Mode, Origin, Record, RequestId, SelectedRecord,
    Selection, SessionId, Summary, Usage,
};
use branch_tally_core::summary::summarize;

struct Rec(SelectedRecord);

fn rec(session: &str, branch: &str) -> Rec {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
        RequestId::new("r"),
        Usage::default(),
    );
    r.model = Some("m".to_owned());
    r.ts = Some("2026-01-01T00:00:00Z".to_owned());
    r.origin = Some(Origin::Main);
    Rec(SelectedRecord {
        attributed: AttributedRecord {
            record: r,
            branch: branch.to_owned(),
            basis: BranchBasis::Stamped,
            subagent: None,
        },
        kept_by: Selection::Exact,
    })
}

impl Rec {
    fn model(mut self, m: Option<&str>) -> Self {
        self.0.attributed.record.model = m.map(str::to_owned);
        self
    }
    fn ts(mut self, t: Option<&str>) -> Self {
        self.0.attributed.record.ts = t.map(str::to_owned);
        self
    }
    fn origin(mut self, o: Option<Origin>) -> Self {
        self.0.attributed.record.origin = o;
        self
    }
    fn side(mut self, agent: Option<&str>) -> Self {
        let r = &mut self.0.attributed.record;
        r.is_sidechain = true;
        r.attribution.agent = agent.map(str::to_owned);
        self
    }
    fn skill(mut self, s: Option<&str>) -> Self {
        self.0.attributed.record.attribution.skill = s.map(str::to_owned);
        self
    }
    fn usage(mut self, u: [u64; 5], think: Option<u64>) -> Self {
        self.0.attributed.record.usage = Usage {
            input: u[0],
            cache_write_5m: u[1],
            cache_write_1h: u[2],
            cache_read: u[3],
            output: u[4],
            thinking: think,
        };
        self
    }
}

fn run(recs: Vec<Rec>) -> Summary {
    let v: Vec<SelectedRecord> = recs.into_iter().map(|r| r.0).collect();
    summarize("feat/x", Mode::Claim, &v, (0, 0), HeuristicCount::default())
}

#[test]
fn carries_branch_and_mode() {
    let s = summarize(
        "feat/x",
        Mode::Strict,
        &[],
        (0, 0),
        HeuristicCount::default(),
    );
    assert_eq!(s.branch, "feat/x");
    assert_eq!(s.mode, Mode::Strict);
    assert!(s.by_model.is_empty() && s.sessions.is_empty());
    assert_eq!(s.total.calls, 0);
}

#[test]
fn model_key_defaults_to_unknown_for_missing_or_empty() {
    let s = run(vec![
        rec("s", "feat/x").model(None),
        rec("s", "feat/x").model(Some("")),
        rec("s", "feat/x").model(Some("m")),
    ]);
    let keys: Vec<_> = s
        .by_model
        .iter()
        .map(|(k, b)| (k.as_str(), b.calls))
        .collect();
    assert_eq!(keys, vec![("unknown", 2), ("m", 1)]);
}

#[test]
fn tables_are_first_seen_ordered() {
    let s = run(vec![
        rec("s2", "feat/x").model(Some("z")),
        rec("s1", "feat/x").model(Some("a")),
        rec("s2", "feat/x").model(Some("z")).side(Some("w")),
        rec("s1", "feat/x").model(Some("a")).skill(Some("k")),
    ]);
    let models: Vec<_> = s.by_model.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(models, ["z", "a"]);
    let attrs: Vec<_> = s.by_attribution.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(attrs, ["main", "agent:w", "k"]);
    let sess: Vec<_> = s.sessions.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(sess, ["s2", "s1"]);
}

#[test]
fn attribution_key_rules_mirror_python_falsiness() {
    let s = run(vec![
        rec("s", "feat/x").skill(Some("sk")).side(Some("ag")),
        rec("s", "feat/x").skill(Some("")).side(Some("ag")),
        rec("s", "feat/x").skill(None).side(Some("")),
        rec("s", "feat/x").skill(None).side(None),
        rec("s", "feat/x").skill(Some("")),
        rec("s", "feat/x"),
    ]);
    let attrs: Vec<_> = s
        .by_attribution
        .iter()
        .map(|(k, b)| (k.as_str(), b.calls))
        .collect();
    assert_eq!(
        attrs,
        vec![
            ("sk", 1),
            ("agent:ag", 1),
            ("agent:subagent", 2),
            ("main", 2)
        ]
    );
}

#[test]
fn buckets_sum_every_field_and_treat_missing_thinking_as_zero() {
    let s = run(vec![
        rec("s", "feat/x").usage([1, 2, 3, 4, 5], Some(6)),
        rec("s", "feat/x").usage([10, 20, 30, 40, 50], None),
    ]);
    let b = &s.by_model[0].1;
    assert_eq!(
        (
            b.input,
            b.cache_write_5m,
            b.cache_write_1h,
            b.cache_read,
            b.output,
            b.thinking,
            b.calls
        ),
        (11, 22, 33, 44, 55, 6, 2)
    );
    assert_eq!(s.total, *b);
    assert_eq!(s.total.tokens(), 165);
}

#[test]
fn total_is_the_sum_over_models() {
    let s = run(vec![
        rec("s", "feat/x")
            .model(Some("a"))
            .usage([1, 0, 0, 0, 2], Some(1)),
        rec("s", "feat/x")
            .model(Some("b"))
            .usage([3, 0, 0, 0, 4], Some(2)),
    ]);
    assert_eq!(
        (
            s.total.input,
            s.total.output,
            s.total.thinking,
            s.total.calls
        ),
        (4, 6, 3, 2)
    );
}

#[test]
fn session_row_tracks_span_tokens_calls_agents_and_first_origin() {
    let s = run(vec![
        rec("s", "feat/x")
            .ts(Some("2026-01-02T00:00:00Z"))
            .usage([1, 0, 0, 0, 1], None),
        rec("s", "feat/x")
            .ts(Some("2026-01-01T00:00:00Z"))
            .origin(Some(Origin::worktree("Wt")))
            .side(Some("b"))
            .usage([2, 0, 0, 0, 2], None),
        rec("s", "feat/x")
            .ts(Some("2026-01-03T00:00:00Z"))
            .side(Some("a")),
    ]);
    let row = &s.sessions[0].1;
    assert_eq!(row.first, "2026-01-01T00:00:00Z");
    assert_eq!(row.last, "2026-01-03T00:00:00Z");
    assert_eq!(row.origin, "main");
    assert_eq!((row.tokens, row.calls), (6, 3));
    assert_eq!(row.who(), "a, b");
}

#[test]
fn missing_timestamp_is_empty_string_and_wins_min_like_python() {
    let s = run(vec![
        rec("s", "feat/x").ts(Some("2026-01-02T00:00:00Z")),
        rec("s", "feat/x").ts(None),
    ]);
    let row = &s.sessions[0].1;
    assert_eq!(row.first, "");
    assert_eq!(row.last, "2026-01-02T00:00:00Z");

    let s = run(vec![
        rec("s", "feat/x").ts(None),
        rec("s", "feat/x").ts(Some("2026-01-02T00:00:00Z")),
    ]);
    assert_eq!(s.sessions[0].1.first, "");
    assert_eq!(s.sessions[0].1.last, "2026-01-02T00:00:00Z");
}

#[test]
fn origin_label_is_lowercased_worktree_slug() {
    let s = run(vec![
        rec("s", "feat/x").origin(Some(Origin::worktree("D--Wt"))),
    ]);
    assert_eq!(s.sessions[0].1.origin, "d--wt");
}

#[test]
fn footnote_counts_sidechain_off_branch_tokens_and_worktree_sessions() {
    let s = run(vec![
        rec("s1", "feat/x").usage([1, 0, 0, 0, 1], None),
        rec("s1", "main").side(None).usage([5, 0, 0, 0, 5], Some(3)),
        rec("s2", "(none)")
            .origin(Some(Origin::worktree("wt")))
            .side(Some("a"))
            .usage([100, 1, 1, 1, 1], None),
        rec("s3", "feat/x").origin(Some(Origin::worktree("wt2"))),
    ]);
    assert_eq!(s.footnote.subagent_calls, 2);
    assert_eq!(s.footnote.off_branch_tokens, 10 + 104);
    assert_eq!(s.footnote.worktree_sessions, 2);
}

#[test]
fn unknown_origin_is_labelled_and_not_counted_as_a_worktree() {
    let s = run(vec![rec("s", "feat/x").origin(None)]);
    assert_eq!(s.sessions[0].1.origin, "unknown");
    assert_eq!(s.footnote.worktree_sessions, 0);
}

fn sub(mut r: Rec, ws: Workspace, name: &str, id: &str) -> Rec {
    r.0.attributed.basis = BranchBasis::DispatcherRolledUp;
    r.0.attributed.subagent = Some(SubagentTags {
        workspace: ws,
        subagent: name.to_owned(),
        agent_id: AgentId::new(id),
    });
    r
}

fn run_with(recs: Vec<Rec>, mode: Mode, un: (u64, u64)) -> Summary {
    let v: Vec<SelectedRecord> = recs.into_iter().map(|r| r.0).collect();
    summarize("feat/x", mode, &v, un, HeuristicCount::default())
}

#[test]
fn subagent_rows_merge_by_workspace_and_name_in_first_seen_order() {
    let recs = vec![
        sub(
            rec("s", "feat/x").usage([10, 0, 0, 0, 0], None),
            Workspace::worktree("wt-a"),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x").usage([0, 0, 0, 0, 5], None),
            Workspace::Main,
            "a9",
            "a9",
        ),
        sub(
            rec("s", "feat/x").usage([1, 0, 0, 0, 1], None),
            Workspace::worktree("WT-A"),
            "writer",
            "a2",
        ),
        rec("s", "feat/x").usage([100, 0, 0, 0, 0], None),
    ];
    let s = run_with(recs, Mode::Claim, (2, 30));
    let rows = &s.subagents.rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].subagent.as_str(), rows[0].calls, rows[0].tokens),
        ("writer", 2, 12)
    );
    assert_eq!(rows[0].agent_ids.len(), 2);
    assert_eq!(
        (rows[1].workspace.clone(), rows[1].calls, rows[1].tokens),
        (Workspace::Main, 1, 5)
    );
    assert_eq!(
        (
            s.subagents.unattributed_calls,
            s.subagents.unattributed_tokens
        ),
        (2, 30)
    );
    assert_eq!(s.total.calls, 4);
}

#[test]
fn subagent_rows_split_by_model_keep_exact_usage_and_by_model_totals_are_unchanged() {
    let wt = || Workspace::worktree("wt-a");
    let recs = vec![
        sub(
            rec("s", "feat/x")
                .model(Some("m1"))
                .usage([10, 1, 2, 3, 4], Some(2)),
            wt(),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x")
                .model(Some("m2"))
                .usage([20, 0, 0, 5, 6], None),
            wt(),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x")
                .model(Some("m1"))
                .usage([1, 0, 0, 0, 1], Some(1)),
            wt(),
            "writer",
            "a2",
        ),
        rec("s", "feat/x")
            .model(Some("m1"))
            .usage([100, 0, 0, 0, 0], None),
    ];
    let s = run_with(recs, Mode::Claim, (0, 0));
    let rows = &s.subagents.rows;
    let models: Vec<&str> = rows.iter().map(|r| r.model.as_str()).collect();
    assert_eq!(models, ["m1", "m2"]);
    let row = &rows[0];
    assert_eq!(row.by_model.len(), 1);
    let m1 = &row.by_model[0].1;
    assert_eq!(
        (
            m1.input,
            m1.cache_write_5m,
            m1.cache_write_1h,
            m1.cache_read
        ),
        (11, 1, 2, 3)
    );
    assert_eq!((m1.output, m1.thinking, m1.calls), (5, 3, 2));
    assert_eq!(rows[1].by_model[0].1.calls, 1);
    // Each row's bucket agrees with its calls/tokens, and the rows sum to the old merged row.
    let t = row.total();
    assert_eq!((t.calls, t.tokens()), (row.calls, row.tokens));
    assert_eq!((t.input, t.output, t.thinking), (11, 5, 3));
    assert_eq!(rows.iter().map(|r| r.total().input).sum::<u64>(), 31);
    assert_eq!(s.subagents.dispatched(), 2);
    // by_model totals are untouched by the breakdown.
    let m = |k: &str| s.by_model.iter().find(|(n, _)| n == k).unwrap().1;
    assert_eq!((m("m1").input, m("m1").calls), (111, 3));
    assert_eq!((m("m2").input, m("m2").calls), (20, 1));
    assert_eq!(s.total.calls, 4);
}

#[test]
fn strict_mode_leaves_subagents_empty() {
    let recs = vec![sub(rec("s", "feat/x"), Workspace::Main, "a", "a")];
    let s = run_with(recs, Mode::Strict, (3, 9));
    assert_eq!(s.subagents, SubagentSummary::default());
}

#[test]
fn rows_hold_only_rolled_up_records_and_unlinked_ones_are_counted_separately() {
    let recs = vec![
        sub(
            rec("s", "feat/x").usage([10, 0, 0, 0, 0], None),
            Workspace::worktree("wt-a"),
            "writer",
            "a1",
        ),
        unlinked(
            rec("s", "feat/x").usage([0, 0, 0, 0, 7], None),
            Workspace::Unknown,
            "n1",
            "n1",
        ),
        unlinked(
            rec("s", "feat/x").usage([3, 0, 0, 0, 0], None),
            Workspace::Unknown,
            "n1",
            "n1",
        ),
    ];
    let s = run_with(recs, Mode::Claim, (2, 10));
    let sub = &s.subagents;
    assert_eq!(sub.rows.len(), 1, "unlinked records must not make rows");
    assert_eq!((sub.calls(), sub.tokens(), sub.dispatched()), (1, 10, 1));
    assert_eq!((sub.unattributed_calls, sub.unattributed_tokens), (2, 10));
    assert_eq!(
        (sub.unlinked_in_total_calls, sub.unlinked_in_total_tokens),
        (2, 10)
    );
    // They stay in the branch total.
    assert_eq!(s.total.calls, 3);
}

#[test]
fn unlinked_in_total_counts_only_selected_records_not_the_repository_wide_figure() {
    // Repository-wide there are 4 unlinked calls, but none was selected for this branch.
    let s = run_with(vec![rec("s", "feat/x")], Mode::Claim, (4, 40));
    assert_eq!(s.subagents.unattributed_calls, 4);
    assert_eq!(s.subagents.unlinked_in_total_calls, 0);
    assert_eq!(s.subagents.unlinked_in_total_tokens, 0);
}

fn unlinked(mut r: Rec, ws: Workspace, name: &str, id: &str) -> Rec {
    r.0.attributed.basis = BranchBasis::SubagentUnlinked;
    r.0.attributed.subagent = Some(SubagentTags {
        workspace: ws,
        subagent: name.to_owned(),
        agent_id: AgentId::new(id),
    });
    r
}

#[test]
fn unattributed_is_reported_even_without_selected_subagents() {
    let s = run_with(vec![rec("s", "feat/x")], Mode::Claim, (4, 40));
    assert_eq!(s.subagents.rows.len(), 0);
    assert_eq!(
        (
            s.subagents.unattributed_calls,
            s.subagents.unattributed_tokens
        ),
        (4, 40)
    );
}

#[test]
fn a_subagent_with_two_models_is_separate_rows_but_one_dispatched_agent_and_same_totals() {
    let wt = || Workspace::worktree("wt-a");
    let recs = vec![
        sub(
            rec("s", "feat/x")
                .model(Some("m1"))
                .usage([10, 0, 0, 0, 1], None),
            wt(),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x")
                .model(Some("m2"))
                .usage([20, 0, 0, 0, 2], None),
            wt(),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x")
                .model(Some("m1"))
                .usage([5, 0, 0, 0, 1], None),
            wt(),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x").model(None).usage([7, 0, 0, 0, 0], None),
            wt(),
            "writer",
            "a1",
        ),
        sub(
            rec("s", "feat/x")
                .model(Some("m1"))
                .usage([1, 0, 0, 0, 0], None),
            Workspace::Main,
            "writer",
            "a2",
        ),
    ];
    let s = run_with(recs, Mode::Claim, (0, 0));
    let sub = &s.subagents;
    let keys: Vec<(String, &str, u64)> = sub
        .rows
        .iter()
        .map(|r| (r.workspace.label(), r.model.as_str(), r.calls))
        .collect();
    assert_eq!(
        keys,
        [
            ("wt-a".to_owned(), "m1", 2),
            ("wt-a".to_owned(), "m2", 1),
            ("wt-a".to_owned(), "unknown", 1),
            ("main".to_owned(), "m1", 1),
        ]
    );
    for r in &sub.rows {
        assert_eq!(r.by_model.len(), 1);
        assert_eq!(r.by_model[0].0, r.model);
        assert_eq!((r.total().calls, r.total().tokens()), (r.calls, r.tokens));
    }
    assert_eq!(
        (sub.calls(), sub.tokens()),
        (5, 10 + 1 + 20 + 2 + 5 + 1 + 7 + 1)
    );
    assert_eq!(
        (sub.dispatched(), sub.workspaces(), sub.worktrees()),
        (2, 2, 1)
    );
}

// ---- heuristic breakdown (#7) ----

use branch_tally_core::model::{HeuristicBreakdown, HeuristicCount};
use branch_tally_core::summary::count_ambiguous;

fn based(mut r: Rec, basis: BranchBasis, kept: Selection) -> Rec {
    r.0.attributed.basis = basis;
    r.0.kept_by = kept;
    r
}

fn hc(calls: u64, tokens: u64) -> HeuristicCount {
    HeuristicCount { calls, tokens }
}

fn run_h(recs: Vec<Rec>, mode: Mode, amb: HeuristicCount) -> Summary {
    let v: Vec<SelectedRecord> = recs.into_iter().map(|r| r.0).collect();
    summarize("feat/x", mode, &v, (0, 0), amb)
}

fn tok(n: u64) -> [u64; 5] {
    [n, 0, 0, 0, 0]
}

#[test]
fn heuristics_count_each_selected_record_under_exactly_one_field() {
    let b = |n, basis, kept| based(rec("s", "feat/x").usage(tok(n), None), basis, kept);
    let recs = vec![
        rec("s", "feat/x").usage(tok(1), None),
        b(2, BranchBasis::Stamped, Selection::SessionClaim),
        b(4, BranchBasis::WorktreeRecovered, Selection::Exact),
        b(8, BranchBasis::OriginRecovered, Selection::Exact),
        b(16, BranchBasis::DispatcherRolledUp, Selection::Exact),
        b(32, BranchBasis::TimelineForward, Selection::Exact),
        b(64, BranchBasis::TimelineCarried, Selection::Exact),
        b(64, BranchBasis::TimelineCarried, Selection::Exact),
    ];
    let h = run_h(recs, Mode::Timeline, hc(0, 0)).footnote.heuristics;
    assert_eq!(h.session_claim, hc(1, 2));
    assert_eq!(h.worktree_recovery, hc(1, 4));
    assert_eq!(h.origin_recovery, hc(1, 8));
    assert_eq!(h.dispatcher_roll_up, hc(1, 16));
    assert_eq!(h.timeline_forward, hc(1, 32));
    assert_eq!(h.timeline_carried, hc(2, 128));
    assert_eq!(h.ambiguous, hc(0, 0));
}

#[test]
fn captured_records_are_counted_under_the_captured_heuristic() {
    let b = |n, kept| {
        based(
            rec("s", "feat/x").usage(tok(n), None),
            BranchBasis::Captured,
            kept,
        )
    };
    let recs = vec![b(3, Selection::Exact), b(4, Selection::Exact)];
    let h = run_h(recs, Mode::Claim, hc(0, 0)).footnote.heuristics;
    assert_eq!(h.captured, hc(2, 7));
    assert_eq!(h.session_claim, hc(0, 0));
    assert!(!h.any_ambiguous());
}

#[test]
fn ambiguous_comes_from_the_argument_and_is_not_double_counted() {
    let recs = vec![based(
        rec("s", "feat/x").usage(tok(5), None),
        BranchBasis::TimelineAmbiguous,
        Selection::Exact,
    )];
    let h = run_h(recs, Mode::Timeline, hc(3, 99)).footnote.heuristics;
    assert_eq!(h.ambiguous, hc(3, 99));
    assert_eq!(h.session_claim, hc(0, 0));
    assert!(h.any_ambiguous());
}

#[test]
fn strict_mode_leaves_heuristics_at_default() {
    let recs = vec![based(
        rec("s", "feat/x"),
        BranchBasis::WorktreeRecovered,
        Selection::Exact,
    )];
    let s = run_h(recs, Mode::Strict, hc(2, 2));
    assert_eq!(s.footnote.heuristics, HeuristicBreakdown::default());
}

#[test]
fn claim_mode_fills_heuristics_too() {
    let recs = vec![based(
        rec("s", "feat/y"),
        BranchBasis::Stamped,
        Selection::SessionClaim,
    )];
    let h = run_h(recs, Mode::Claim, hc(0, 0)).footnote.heuristics;
    assert_eq!(h.session_claim.calls, 1);
}

#[test]
fn count_ambiguous_covers_selected_sessions_selected_or_not() {
    let r = |s, br, n, basis| based(rec(s, br).usage(tok(n), None), basis, Selection::Exact);
    let amb = BranchBasis::TimelineAmbiguous;
    let selected_rec = r("s1", "feat/x", 1, BranchBasis::Stamped);
    let amb_sel = r("s1", "feat/x", 10, amb);
    let amb_unsel = r("s1", "other", 20, amb);
    let unlinked = r("s1", "other", 40, BranchBasis::SubagentUnlinked);
    let other_session = r("s2", "other", 80, amb);
    let plain_unsel = r("s1", "other", 160, BranchBasis::Stamped);

    let all: Vec<AttributedRecord> = [
        &selected_rec,
        &amb_sel,
        &amb_unsel,
        &unlinked,
        &other_session,
        &plain_unsel,
    ]
    .iter()
    .map(|x| x.0.attributed.clone())
    .collect();
    let selected = vec![selected_rec.0.clone(), amb_sel.0.clone()];
    assert_eq!(count_ambiguous(&all, &selected), hc(3, 70));
}
