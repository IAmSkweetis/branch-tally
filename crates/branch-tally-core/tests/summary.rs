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
    summarize("feat/x", Mode::Claim, &v, (0, 0))
}

#[test]
fn carries_branch_and_mode() {
    let s = summarize("feat/x", Mode::Strict, &[], (0, 0));
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
    summarize("feat/x", mode, &v, un)
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
