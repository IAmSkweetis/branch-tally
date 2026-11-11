//! Attribution: worktree recovery and record selection, ported from `reference/token_report.py`.

use std::path::Path;

use branch_tally_core::attribution::{WorktreeMap, attribute, select};
use branch_tally_core::model::{
    AttributedRecord, BranchBasis, Harness, Origin, Record, RequestId, Selection, SessionId, Usage,
};

fn rec(session: &str, stamp: Option<&str>, origin: Option<Origin>) -> Record {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
        RequestId::new(format!("req-{session}-{}", stamp.unwrap_or("none"))),
        Usage::default(),
    );
    r.git_branch = stamp.map(str::to_owned);
    r.origin = origin;
    r
}

fn map() -> WorktreeMap {
    WorktreeMap::new(
        Path::new(r"D:\work\Repo"),
        [
            (Path::new(r"D:\work\Repo"), "main"),
            (
                Path::new(r"D:\work\Repo\.claude\worktrees\agent-abc"),
                "feat/real",
            ),
            (
                Path::new("/home/me/repo/.claude/worktrees/agent-posix"),
                "fix/posix",
            ),
        ],
    )
}

fn one(r: Record, strict: bool) -> (String, BranchBasis) {
    let a = attribute([r], &[], &map(), strict).remove(0);
    (a.branch, a.basis)
}

fn origin_of(map: &WorktreeMap, slug: &str) -> (String, BranchBasis) {
    let a = attribute(
        [rec("s", None, Some(Origin::Worktree(slug.to_owned())))],
        &[],
        map,
        false,
    )
    .remove(0);
    (a.branch, a.basis)
}

#[test]
fn git_bash_main_checkout_recovers_worktree_origin_in_either_slug_form() {
    // Main checkout arrives as /d/x; git reports the worktree as D:\x\...
    let m = WorktreeMap::new(
        Path::new("/d/x"),
        [(Path::new(r"D:\x\.claude\worktrees\agent-a"), "feat/a")],
    );
    for slug in [
        "D--x--claude-worktrees-agent-a",
        "-d-x--claude-worktrees-agent-a",
    ] {
        assert_eq!(
            origin_of(&m, slug),
            ("feat/a".to_owned(), BranchBasis::OriginRecovered),
            "{slug}"
        );
    }
}

#[test]
fn git_bash_worktree_path_recovers_drive_form_origin() {
    let m = WorktreeMap::new(
        Path::new(r"D:\x"),
        [(Path::new("/d/x/.claude/worktrees/agent-a"), "feat/a")],
    );
    assert_eq!(
        origin_of(&m, "D--x--claude-worktrees-agent-a"),
        ("feat/a".to_owned(), BranchBasis::OriginRecovered)
    );
}

#[test]
fn verbatim_unc_worktree_keys_as_plain_unc_slug() {
    let m = WorktreeMap::new(
        Path::new(r"\\?\UNC\srv\share\r"),
        [(
            Path::new(r"\\?\UNC\srv\share\r\.claude\worktrees\agent-u"),
            "feat/u",
        )],
    );
    assert_eq!(
        m.get("--srv-share-r--claude-worktrees-agent-u"),
        Some("feat/u")
    );
    assert_eq!(m.get("unc-srv-share-r--claude-worktrees-agent-u"), None);
}

#[test]
fn main_checkout_stays_unrecovered_for_git_bash_main() {
    let m = WorktreeMap::new(Path::new("/d/x"), [(Path::new(r"D:\x"), "main")]);
    assert_eq!(
        origin_of(&m, "main"),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn worktree_agent_stamp_is_remapped() {
    let (b, basis) = one(
        rec("s", Some("worktree-agent-abc"), Some(Origin::Main)),
        false,
    );
    assert_eq!(
        (b.as_str(), basis),
        ("feat/real", BranchBasis::WorktreeRecovered)
    );
}

#[test]
fn worktree_agent_stamp_remaps_from_posix_path_too() {
    let (b, basis) = one(rec("s", Some("worktree-agent-posix"), None), false);
    assert_eq!(
        (b.as_str(), basis),
        ("fix/posix", BranchBasis::WorktreeRecovered)
    );
}

#[test]
fn unknown_worktree_agent_stamp_stays_stamped() {
    let (b, basis) = one(rec("s", Some("worktree-agent-gone"), None), false);
    assert_eq!(
        (b.as_str(), basis),
        ("worktree-agent-gone", BranchBasis::Stamped)
    );
}

#[test]
fn none_stamp_recovers_from_worktree_origin_case_insensitively() {
    let origin = Origin::worktree("D--work-Repo--claude-worktrees-Agent-ABC");
    let (b, basis) = one(rec("s", None, Some(origin)), false);
    assert_eq!(
        (b.as_str(), basis),
        ("feat/real", BranchBasis::OriginRecovered)
    );
}

#[test]
fn posix_origin_slug_recovers() {
    let origin = Origin::worktree("-home-me-repo--claude-worktrees-agent-posix");
    let (b, basis) = one(rec("s", None, Some(origin)), false);
    assert_eq!(
        (b.as_str(), basis),
        ("fix/posix", BranchBasis::OriginRecovered)
    );
}

#[test]
fn empty_stamp_counts_as_none() {
    let origin = Origin::worktree("d--work-repo--claude-worktrees-agent-abc");
    let (b, basis) = one(rec("s", Some(""), Some(origin)), false);
    assert_eq!(
        (b.as_str(), basis),
        ("feat/real", BranchBasis::OriginRecovered)
    );
}

#[test]
fn none_without_recoverable_origin_is_the_literal_none() {
    assert_eq!(
        one(rec("s", None, None), false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
    let unknown = Origin::worktree("d--other--claude-worktrees-x");
    assert_eq!(
        one(rec("s", None, Some(unknown)), false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn real_stamp_is_never_replaced_by_origin() {
    let origin = Origin::worktree("d--work-repo--claude-worktrees-agent-abc");
    let (b, basis) = one(rec("s", Some("feat/mine"), Some(origin)), false);
    assert_eq!((b.as_str(), basis), ("feat/mine", BranchBasis::Stamped));
}

#[test]
fn unstamped_main_checkout_record_is_not_recovered_onto_the_checkouts_current_branch() {
    // #37: the reference also looks the main checkout up in the worktree list, which only says
    // what is checked out *now*. The record stays `(none)` and is reported as unattributed.
    let (b, basis) = one(rec("s", None, Some(Origin::Main)), false);
    assert_eq!((b.as_str(), basis), ("(none)", BranchBasis::Stamped));
}

#[test]
fn strict_disables_both_recoveries() {
    let a = rec("s", Some("worktree-agent-abc"), Some(Origin::Main));
    assert_eq!(
        one(a, true),
        ("worktree-agent-abc".to_owned(), BranchBasis::Stamped)
    );
    let origin = Origin::worktree("d--work-repo--claude-worktrees-agent-abc");
    assert_eq!(
        one(rec("s", None, Some(origin)), true),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn attribute_keeps_every_record_in_order() {
    let rs = vec![
        rec("a", Some("x"), None),
        rec("b", None, None),
        rec("c", Some("y"), None),
    ];
    let out = attribute(rs, &[], &map(), false);
    let sessions: Vec<_> = out.iter().map(|a| a.record.session_id.as_str()).collect();
    assert_eq!(sessions, ["a", "b", "c"]);
}

fn attributed(rs: Vec<Record>) -> Vec<AttributedRecord> {
    attribute(rs, &[], &WorktreeMap::default(), true)
}

#[test]
fn strict_select_is_exact_branch_equality() {
    let rs = attributed(vec![
        rec("s1", Some("feat/a"), None),
        rec("s1", Some("main"), None),
        rec("s2", Some("feat/a"), None),
    ]);
    let out = select(rs, "feat/a", true);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|s| s.kept_by == Selection::Exact));
}

#[test]
fn claim_pulls_in_the_rest_of_a_session() {
    let rs = attributed(vec![
        rec("s1", Some("main"), None),
        rec("s1", Some("feat/a"), None),
        rec("s1", None, None),
        rec("s2", Some("main"), None),
    ]);
    let out = select(rs, "feat/a", false);
    let kept: Vec<_> = out
        .iter()
        .map(|s| (s.attributed.branch.as_str(), s.kept_by))
        .collect();
    assert_eq!(
        kept,
        [
            ("main", Selection::SessionClaim),
            ("feat/a", Selection::Exact),
            ("(none)", Selection::SessionClaim),
        ]
    );
}

#[test]
fn two_feature_branches_in_a_session_stay_ambiguous() {
    let rs = attributed(vec![
        rec("s1", Some("main"), None),
        rec("s1", Some("feat/a"), None),
        rec("s1", Some("fix/b"), None),
    ]);
    let out = select(rs, "feat/a", false);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kept_by, Selection::Exact);
}

#[test]
fn non_feature_branch_does_not_claim() {
    let rs = attributed(vec![
        rec("s1", Some("main"), None),
        rec("s1", Some("develop"), None),
    ]);
    let out = select(rs, "develop", false);
    assert_eq!(out.len(), 1);
}

#[test]
fn feature_prefixes_match_only_with_slash() {
    for p in [
        "feat", "feature", "fix", "bugfix", "hotfix", "chore", "release",
    ] {
        let b = format!("{p}/x");
        let rs = attributed(vec![rec("s", Some("main"), None), rec("s", Some(&b), None)]);
        assert_eq!(select(rs, &b, false).len(), 2, "{p}");
    }
    let rs = attributed(vec![
        rec("s", Some("main"), None),
        rec("s", Some("featx/y"), None),
    ]);
    assert_eq!(select(rs, "featx/y", false).len(), 1);
}

#[test]
fn recovered_branches_feed_the_claim() {
    let rs = attribute(
        [
            rec("s1", Some("worktree-agent-abc"), None),
            rec("s1", Some("main"), None),
        ],
        &[],
        &map(),
        false,
    );
    let out = select(rs, "feat/real", false);
    assert_eq!(out.len(), 2);
}

// ---- dispatcher roll-up (#6) ----

use branch_tally_core::model::{AgentId, Dispatch, Workspace};

const WT_BACKSLASH: &str = r"D:\work\Repo\.claude\worktrees";
const WT_POSIX: &str = "/d/work/Repo/.claude/worktrees";

fn sub(agent: &str, stamp: Option<&str>, cwd: Option<&str>) -> Record {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new("orch"),
        RequestId::new(format!("req-sub-{agent}-{}", stamp.unwrap_or("none"))),
        Usage::default(),
    );
    r.agent_id = Some(AgentId::new(agent));
    r.parent_session = Some(SessionId::new("orch"));
    r.git_branch = stamp.map(str::to_owned);
    r.cwd = cwd.map(str::to_owned);
    r
}

fn dispatch(agent: &str, branch: Option<&str>) -> Dispatch {
    Dispatch {
        session_id: SessionId::new("orch"),
        agent_id: AgentId::new(agent),
        git_branch: branch.map(str::to_owned),
        ts: None,
    }
}

fn run(rs: Vec<Record>, ds: &[Dispatch], strict: bool) -> Vec<AttributedRecord> {
    attribute(rs, ds, &map(), strict)
}

#[test]
fn three_worktree_subagents_with_gone_worktrees_roll_up_to_the_dispatcher() {
    let rs = vec![
        sub(
            "a1",
            Some("worktree-agent-a1"),
            Some(&format!(r"{WT_BACKSLASH}\agent-a1")),
        ),
        sub(
            "a2",
            Some("worktree-agent-a2"),
            Some(&format!(r"{WT_BACKSLASH}\agent-a2")),
        ),
        sub("a3", None, Some(&format!("{WT_POSIX}/agent-a3"))),
    ];
    let ds = [
        dispatch("a1", Some("feat/x")),
        dispatch("a2", Some("feat/x")),
        dispatch("a3", Some("feat/x")),
    ];
    let out = run(rs, &ds, false);
    for a in &out {
        assert_eq!(a.branch, "feat/x");
        assert_eq!(a.basis, BranchBasis::DispatcherRolledUp);
    }
    let ws: Vec<_> = out
        .iter()
        .map(|a| a.subagent.as_ref().unwrap().workspace.clone())
        .collect();
    assert_eq!(
        ws,
        [
            Workspace::worktree("agent-a1"),
            Workspace::worktree("agent-a2"),
            Workspace::worktree("agent-a3")
        ]
    );
}

#[test]
fn live_worktree_holding_a_renamed_branch_does_not_win() {
    // agent-abc still exists in map() and holds feat/real; the dispatcher was on feat/x.
    let rs = vec![sub(
        "abc",
        Some("worktree-agent-abc"),
        Some(&format!(r"{WT_BACKSLASH}\agent-abc")),
    )];
    let out = run(rs, &[dispatch("abc", Some("feat/x"))], false);
    assert_eq!(out[0].branch, "feat/x");
    assert_eq!(out[0].basis, BranchBasis::DispatcherRolledUp);
}

#[test]
fn stale_own_stamp_is_overridden_by_the_dispatcher() {
    let rs = vec![sub("a1", Some("fix/other"), None)];
    let out = run(rs, &[dispatch("a1", Some("feat/x"))], false);
    assert_eq!(out[0].branch, "feat/x");
}

#[test]
fn unusable_dispatcher_stamp_or_no_link_is_unlinked_with_the_raw_stamp() {
    let cwd = format!(r"{WT_BACKSLASH}\agent-abc");
    let rs = vec![
        sub("m", Some("worktree-agent-abc"), Some(&cwd)),
        sub("w", Some("worktree-agent-abc"), Some(&cwd)),
        sub("n", Some("worktree-agent-abc"), Some(&cwd)),
        sub("e", None, Some(&cwd)),
    ];
    let ds = [
        dispatch("m", None),
        dispatch("w", Some("worktree-agent-zzz")),
        dispatch("e", Some("")),
    ];
    let out = run(rs, &ds, false);
    // None of these may take the live `git worktree list` lookup (agent-abc exists in map()).
    let got: Vec<_> = out.iter().map(|a| (a.branch.as_str(), a.basis)).collect();
    assert_eq!(
        got,
        [
            ("worktree-agent-abc", BranchBasis::SubagentUnlinked),
            ("worktree-agent-abc", BranchBasis::SubagentUnlinked),
            ("worktree-agent-abc", BranchBasis::SubagentUnlinked),
            ("(none)", BranchBasis::SubagentUnlinked),
        ]
    );
    assert!(out.iter().all(|a| a.subagent.is_some()));
}

#[test]
fn unlinked_subagent_does_not_take_origin_recovery() {
    let mut r = sub("n", None, None);
    r.origin = Some(Origin::Main);
    let out = run(vec![r], &[], false);
    assert_eq!(
        (out[0].branch.as_str(), out[0].basis),
        ("(none)", BranchBasis::SubagentUnlinked)
    );
}

#[test]
fn dispatch_from_another_session_is_not_a_link() {
    let mut d = dispatch("a1", Some("feat/x"));
    d.session_id = SessionId::new("someone-else");
    let out = run(vec![sub("a1", Some("fix/y"), None)], &[d], false);
    assert_eq!(
        (out[0].branch.as_str(), out[0].basis),
        ("fix/y", BranchBasis::SubagentUnlinked)
    );
}

#[test]
fn two_subagents_in_one_worktree_share_the_workspace_and_differ_in_name() {
    let cwd = format!("{WT_POSIX}/agent-a1/crates/core");
    let mut named = sub("a1", None, Some(&cwd));
    named.attribution.agent = Some("Explore".to_owned());
    let unnamed = sub("a2", None, Some(&cwd.to_uppercase()));
    let ds = [
        dispatch("a1", Some("feat/x")),
        dispatch("a2", Some("feat/x")),
    ];
    let out = run(vec![named, unnamed], &ds, false);
    let t0 = out[0].subagent.clone().unwrap();
    let t1 = out[1].subagent.clone().unwrap();
    assert_eq!(t0.workspace, Workspace::worktree("agent-a1"));
    assert_eq!(t1.workspace, t0.workspace);
    assert_eq!(t0.subagent, "Explore");
    assert_eq!(t1.subagent, "a2");
    assert_eq!(t0.agent_id, AgentId::new("a1"));
}

#[test]
fn non_worktree_subagent_is_in_the_main_workspace_in_both_cwd_forms() {
    let rs = vec![
        sub("a1", None, Some(r"D:\work\Repo\crates")),
        sub("a2", None, Some("/d/work/repo")),
    ];
    let ds = [
        dispatch("a1", Some("feat/x")),
        dispatch("a2", Some("feat/x")),
    ];
    let out = run(rs, &ds, false);
    for a in &out {
        assert_eq!(a.subagent.as_ref().unwrap().workspace, Workspace::Main);
        assert_eq!(a.basis, BranchBasis::DispatcherRolledUp);
    }
}

#[test]
fn cwd_missing_or_outside_the_repository_still_rolls_up_but_is_not_tagged_main() {
    let rs = vec![
        sub("a1", None, None),
        sub("a2", None, Some(r"C:\elsewhere\proj")),
        sub("a3", None, Some(r"D:\work\Repo-sibling")),
    ];
    let ds = [
        dispatch("a1", Some("feat/x")),
        dispatch("a2", Some("feat/x")),
        dispatch("a3", Some("feat/x")),
    ];
    let out = run(rs, &ds, false);
    for a in &out {
        assert_eq!(a.branch, "feat/x");
        assert_eq!(a.basis, BranchBasis::DispatcherRolledUp);
        let tags = a.subagent.as_ref().expect("tagged");
        assert_eq!(tags.workspace, Workspace::Unknown);
        assert_ne!(tags.workspace, Workspace::Main);
    }
}

#[test]
fn strict_mode_ignores_dispatches_and_is_unchanged() {
    let rs = vec![sub(
        "abc",
        Some("worktree-agent-abc"),
        Some(&format!(r"{WT_BACKSLASH}\agent-abc")),
    )];
    let out = run(rs, &[dispatch("abc", Some("feat/x"))], true);
    assert_eq!(
        (out[0].branch.as_str(), out[0].basis),
        ("worktree-agent-abc", BranchBasis::Stamped)
    );
    assert!(out[0].subagent.is_none());
}

#[test]
fn non_subagent_records_are_untouched_by_dispatches() {
    let out = run(
        vec![rec("orch", Some("worktree-agent-abc"), None)],
        &[dispatch("abc", Some("feat/x"))],
        false,
    );
    assert_eq!(
        (out[0].branch.as_str(), out[0].basis),
        ("feat/real", BranchBasis::WorktreeRecovered)
    );
    assert!(out[0].subagent.is_none());
}

#[test]
fn claim_counts_each_record_once_with_rolled_up_and_unlinked_in_one_session() {
    let rs = vec![
        rec("orch", Some("main"), None),
        sub("a1", Some("worktree-agent-a1"), None),
        sub("a2", Some("worktree-agent-a2"), None),
    ];
    let out = run(rs, &[dispatch("a1", Some("feat/x"))], false);
    let sel = select(out, "feat/x", false);
    // The orchestrator's `main` record is claimed (existing heuristic); rolled-up a1 is Exact;
    // unlinked a2 shares the session so the claim keeps it too: counted, not dropped.
    assert_eq!(sel.len(), 3);
    assert_eq!(
        sel.iter().filter(|s| s.kept_by == Selection::Exact).count(),
        1
    );
}

// ---- timeline mode (#7) ----

use branch_tally_core::attribution::{attribute_timeline, default_work_branch, select_timeline};

fn trec(session: &str, n: usize, ts: Option<&str>, stamp: Option<&str>) -> Record {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new(session),
        RequestId::new(format!("req-{session}-{n}")),
        Usage::default(),
    );
    r.git_branch = stamp.map(str::to_owned);
    r.ts = ts.map(str::to_owned);
    r
}

fn t(min: usize) -> &'static str {
    // Fixed table so each test reads as a sequence of minutes.
    const T: [&str; 8] = [
        "2026-11-01T10:00:00.000Z",
        "2026-11-01T10:01:00.000Z",
        "2026-11-01T10:02:00.000Z",
        "2026-11-01T10:03:00.000Z",
        "2026-11-01T10:04:00.000Z",
        "2026-11-01T10:05:00.000Z",
        "2026-11-01T10:06:00.000Z",
        "2026-11-01T10:07:00.000Z",
    ];
    T[min]
}

fn tl(rs: Vec<Record>, ds: &[Dispatch]) -> Vec<(String, BranchBasis)> {
    let work = default_work_branch("main");
    attribute_timeline(rs, ds, &map(), &work)
        .into_iter()
        .map(|a| (a.branch, a.basis))
        .collect()
}

fn pair(b: &str, basis: BranchBasis) -> (String, BranchBasis) {
    (b.to_owned(), basis)
}

#[test]
fn default_predicate_classifies_branches() {
    let w = default_work_branch("main");
    for yes in [
        "feat/a",
        "fix/b",
        "refactor/c",
        "docs/d",
        "release",
        "trunk",
    ] {
        assert!(w(yes), "{yes} should be work");
    }
    for no in ["main", "HEAD", "", "worktree-agent-abc"] {
        assert!(!w(no), "{no:?} should not be work");
    }
    assert!(default_work_branch("trunk")("main"));
    assert!(!default_work_branch("trunk")("trunk"));
}

#[test]
fn timeline_single_branch_session_is_all_stamped() {
    let rs = (0..3)
        .map(|i| trec("s", i, Some(t(i)), Some("feat/a")))
        .collect();
    let out = tl(rs, &[]);
    assert!(
        out.iter()
            .all(|p| *p == pair("feat/a", BranchBasis::Stamped))
    );
}

#[test]
fn timeline_forward_carried_and_switch() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("main")),
        trec("s", 1, Some(t(1)), Some("feat/a")),
        trec("s", 2, Some(t(2)), Some("main")),
        trec("s", 3, Some(t(3)), Some("fix/b")),
        trec("s", 4, Some(t(4)), Some("main")),
    ];
    assert_eq!(
        tl(rs, &[]),
        vec![
            pair("feat/a", BranchBasis::TimelineForward),
            pair("feat/a", BranchBasis::Stamped),
            pair("feat/a", BranchBasis::TimelineCarried),
            pair("fix/b", BranchBasis::Stamped),
            pair("fix/b", BranchBasis::TimelineCarried),
        ]
    );
}

#[test]
fn timeline_input_order_does_not_matter_but_output_order_is_kept() {
    let rs = vec![
        trec("s", 0, Some(t(2)), Some("main")),
        trec("s", 1, Some(t(1)), Some("feat/a")),
    ];
    assert_eq!(
        tl(rs, &[]),
        vec![
            pair("feat/a", BranchBasis::TimelineCarried),
            pair("feat/a", BranchBasis::Stamped)
        ]
    );
}

#[test]
fn timeline_refactor_and_docs_are_work_branches() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("refactor/x")),
        trec("s", 1, Some(t(1)), Some("main")),
        trec("s", 2, Some(t(2)), Some("docs/y")),
    ];
    let out = tl(rs, &[]);
    assert_eq!(out[1], pair("refactor/x", BranchBasis::TimelineCarried));
    assert_eq!(out[2], pair("docs/y", BranchBasis::Stamped));
}

#[test]
fn timeline_worktree_agent_stamp_is_carried_and_never_looked_up() {
    // `map()` knows worktree-agent-abc as feat/real; timeline must ignore that.
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("feat/a")),
        trec("s", 1, Some(t(1)), Some("worktree-agent-abc")),
    ];
    assert_eq!(tl(rs, &[])[1], pair("feat/a", BranchBasis::TimelineCarried));
    // Even a permissive predicate cannot make a worktree-agent stamp a work branch.
    let all = |_: &str| true;
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("feat/a")),
        trec("s", 1, Some(t(1)), Some("worktree-agent-abc")),
    ];
    let out = attribute_timeline(rs, &[], &map(), &all);
    assert_eq!(out[1].branch, "feat/a");
}

#[test]
fn timeline_missing_stamp_is_non_work_and_origin_is_not_recovered() {
    let mut r = trec("s", 1, Some(t(1)), None);
    r.origin = Some(Origin::Main);
    let rs = vec![trec("s", 0, Some(t(0)), Some("feat/a")), r];
    assert_eq!(tl(rs, &[])[1], pair("feat/a", BranchBasis::TimelineCarried));
}

#[test]
fn timeline_session_with_only_main_is_not_attributed() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("main")),
        trec("s", 1, Some(t(1)), None),
        trec("s", 2, None, Some("main")),
    ];
    assert_eq!(
        tl(rs, &[]),
        vec![
            pair("main", BranchBasis::Stamped),
            pair("(none)", BranchBasis::Stamped),
            pair("main", BranchBasis::Stamped),
        ]
    );
}

#[test]
fn timeline_sessions_are_independent() {
    let rs = vec![
        trec("a", 0, Some(t(0)), Some("feat/a")),
        trec("b", 0, Some(t(1)), Some("main")),
        trec("b", 1, Some(t(2)), Some("fix/b")),
    ];
    let out = tl(rs, &[]);
    assert_eq!(out[1], pair("fix/b", BranchBasis::TimelineForward));
}

#[test]
fn timeline_missing_or_unparseable_ts_is_ambiguous_and_keeps_its_stamp() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("feat/a")),
        trec("s", 1, None, Some("main")),
        trec("s", 2, Some("not-a-time"), Some("fix/b")),
    ];
    let out = tl(rs, &[]);
    assert_eq!(out[1], pair("main", BranchBasis::TimelineAmbiguous));
    assert_eq!(out[2], pair("fix/b", BranchBasis::TimelineAmbiguous));
    assert_eq!(out[0], pair("feat/a", BranchBasis::Stamped));
}

#[test]
fn timeline_same_ts_on_different_work_branches_is_ambiguous() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("feat/a")),
        trec("s", 1, Some(t(1)), Some("feat/a")),
        trec("s", 2, Some(t(1)), Some("fix/b")),
        trec("s", 3, Some(t(2)), Some("main")),
    ];
    let out = tl(rs, &[]);
    assert_eq!(out[1], pair("feat/a", BranchBasis::TimelineAmbiguous));
    assert_eq!(out[2], pair("fix/b", BranchBasis::TimelineAmbiguous));
    // The conflict does not move the active branch: the later record is carried from feat/a.
    assert_eq!(out[3], pair("feat/a", BranchBasis::TimelineCarried));
}

#[test]
fn timeline_same_ts_same_branch_is_fine() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("feat/a")),
        trec("s", 1, Some(t(0)), Some("feat/a")),
    ];
    assert!(tl(rs, &[]).iter().all(|p| p.1 == BranchBasis::Stamped));
}

#[test]
fn timeline_ts_with_offsets_compares_by_instant() {
    let rs = vec![
        trec("s", 0, Some("2026-11-01T10:30:00+01:00"), Some("feat/a")),
        trec("s", 1, Some("2026-11-01T09:45:00Z"), Some("main")),
        trec("s", 2, Some("2026-11-01T09:15:00Z"), Some("main")),
    ];
    let out = tl(rs, &[]);
    // 10:30+01:00 is 09:30Z: record 2 (09:15Z) is before it, record 1 (09:45Z) after.
    assert_eq!(out[1], pair("feat/a", BranchBasis::TimelineCarried));
    assert_eq!(out[2], pair("feat/a", BranchBasis::TimelineForward));
}

fn tdispatch(agent: &str, stamp: Option<&str>, ts: Option<&str>) -> Dispatch {
    Dispatch {
        session_id: SessionId::new("orch"),
        agent_id: AgentId::new(agent),
        git_branch: stamp.map(str::to_owned),
        ts: ts.map(str::to_owned),
    }
}

fn orch(n: usize, minute: usize, stamp: &str) -> Record {
    trec("orch", n, Some(t(minute)), Some(stamp))
}

#[test]
fn timeline_subagent_takes_the_dispatchers_timeline_branch_not_the_dispatch_stamp() {
    let cwd = format!("{WT_BACKSLASH}\\agent-zzz");
    let s1 = sub("a1", Some("worktree-agent-zzz"), Some(&cwd));
    let s2 = sub("a2", Some("main"), None);
    let rs = vec![
        orch(0, 0, "main"),
        orch(1, 1, "feat/a"),
        orch(2, 2, "main"),
        s1,
        s2,
    ];
    let ds = [
        tdispatch("a1", Some("worktree-agent-zzz"), Some(t(2))),
        tdispatch("a2", Some("main"), Some(t(3))),
    ];
    let work = default_work_branch("main");
    let out = attribute_timeline(rs, &ds, &map(), &work);
    assert_eq!(out[3].branch, "feat/a");
    assert_eq!(out[3].basis, BranchBasis::DispatcherRolledUp);
    assert_eq!(out[4].branch, "feat/a");
    assert_eq!(out[4].basis, BranchBasis::DispatcherRolledUp);
    let tags = out[3].subagent.as_ref().expect("tags");
    assert_eq!(tags.workspace, Workspace::Worktree("agent-zzz".into()));
}

#[test]
fn timeline_subagent_follows_the_timeline_at_dispatch_time_not_later() {
    let rs = vec![
        orch(0, 0, "feat/a"),
        orch(1, 3, "fix/b"),
        sub("a1", None, None),
    ];
    let ds = [tdispatch("a1", Some("main"), Some(t(1)))];
    let work = default_work_branch("main");
    let out = attribute_timeline(rs, &ds, &map(), &work);
    assert_eq!(out[2].branch, "feat/a");
    assert_eq!(out[2].basis, BranchBasis::DispatcherRolledUp);
}

#[test]
fn timeline_subagent_without_a_usable_link_is_unlinked_and_keeps_its_stamp() {
    let mut no_work_parent = sub("nowork", Some("fix/own"), None);
    no_work_parent.parent_session = Some(SessionId::new("other"));
    let rs = vec![
        orch(0, 0, "feat/a"),
        sub("none", Some("worktree-agent-q"), None),
        sub("nots", Some("fix/own"), None),
        sub("badts", Some("fix/own"), None),
        no_work_parent,
        trec("other", 0, Some(t(0)), Some("main")),
    ];
    let ds = vec![
        tdispatch("nots", Some("main"), None),
        tdispatch("badts", Some("main"), Some("garbage")),
        Dispatch {
            session_id: SessionId::new("other"),
            agent_id: AgentId::new("nowork"),
            git_branch: None,
            ts: Some(t(0).to_owned()),
        },
    ];
    let work = default_work_branch("main");
    let out = attribute_timeline(rs, &ds, &map(), &work);
    assert_eq!(out[1].branch, "worktree-agent-q");
    for (i, a) in out.iter().enumerate().take(5).skip(1) {
        assert_eq!(a.basis, BranchBasis::SubagentUnlinked, "record {i}");
    }
    assert_eq!(out[2].branch, "fix/own");
    assert!(out[1].subagent.is_some());
}

#[test]
fn timeline_subagent_dispatched_inside_a_conflict_is_unlinked() {
    let rs = vec![
        orch(0, 0, "feat/a"),
        orch(1, 1, "feat/a"),
        orch(2, 1, "fix/b"),
        sub("a1", None, None),
    ];
    let ds = [tdispatch("a1", Some("feat/a"), Some(t(1)))];
    let work = default_work_branch("main");
    let out = attribute_timeline(rs, &ds, &map(), &work);
    assert_eq!(out[3].basis, BranchBasis::SubagentUnlinked);
}

#[test]
fn timeline_subagent_records_do_not_move_the_dispatchers_timeline() {
    let rs = vec![
        orch(0, 0, "feat/a"),
        sub("a1", Some("fix/own"), None),
        orch(1, 3, "main"),
    ];
    let ds = [tdispatch("a1", None, Some(t(1)))];
    let work = default_work_branch("main");
    let out = attribute_timeline(rs, &ds, &map(), &work);
    assert_eq!(out[2].branch, "feat/a");
    assert_eq!(out[2].basis, BranchBasis::TimelineCarried);
}

#[test]
fn timeline_keeps_every_record_in_order() {
    let rs: Vec<Record> = (0..6)
        .map(|i| {
            let ts = if i % 2 == 0 { Some(t(i)) } else { None };
            trec("s", i, ts, Some("feat/a"))
        })
        .collect();
    let ids: Vec<String> = rs.iter().map(|r| r.request_id.to_string()).collect();
    let work = default_work_branch("main");
    let out = attribute_timeline(rs, &[], &map(), &work);
    let got: Vec<String> = out
        .iter()
        .map(|a| a.record.request_id.to_string())
        .collect();
    assert_eq!(got, ids);
}

#[test]
fn select_timeline_is_exact_equality_and_never_claims() {
    let rs = vec![
        trec("s", 0, Some(t(0)), Some("feat/a")),
        trec("s", 1, Some(t(1)), Some("main")),
        trec("s", 2, Some(t(2)), Some("fix/b")),
        trec("t", 0, Some(t(0)), Some("main")),
    ];
    let work = default_work_branch("main");
    let out = select_timeline(attribute_timeline(rs, &[], &map(), &work), "feat/a");
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|s| s.kept_by == Selection::Exact));
}

#[test]
fn claim_and_strict_are_unchanged_by_timeline_support() {
    let (b, basis) = one(rec("s", Some("worktree-agent-abc"), None), false);
    assert_eq!(
        (b.as_str(), basis),
        ("feat/real", BranchBasis::WorktreeRecovered)
    );
    let sel = select(
        attributed(vec![
            rec("s", Some("feat/a"), None),
            rec("s", Some("main"), None),
        ]),
        "feat/a",
        false,
    );
    assert_eq!(sel.len(), 2);
}

// ---- captured attribution (#13) ----

use branch_tally_core::attribution::attribute_with_captures;
use branch_tally_core::model::{CAPTURE_SCHEMA_VERSION, CaptureEvent, CaptureKind};

fn cap(session: &str, kind: CaptureKind, branch: &str, ts: &str) -> CaptureEvent {
    CaptureEvent {
        v: CAPTURE_SCHEMA_VERSION,
        harness: Harness::ClaudeCode,
        kind,
        session_id: SessionId::new(session),
        agent_id: None,
        branch: branch.to_owned(),
        worktree_branch: None,
        ts: ts.to_owned(),
    }
}

fn at(mut r: Record, ts: Option<&str>) -> Record {
    r.ts = ts.map(str::to_owned);
    r
}

fn with_caps(r: Record, caps: &[CaptureEvent], strict: bool) -> (String, BranchBasis) {
    let a = attribute_with_captures([r], &[], caps, &map(), strict).remove(0);
    (a.branch, a.basis)
}

const T0: &str = "2026-11-01T10:00:00Z";
const T1: &str = "2026-11-01T11:00:00Z";
const T2: &str = "2026-11-01T12:00:00Z";

#[test]
fn bracketing_captures_that_agree_recover_a_missing_stamp() {
    let caps = [
        cap("s", CaptureKind::SessionStart, "feat/cap", T0),
        cap("s", CaptureKind::Stop, "feat/cap", T2),
    ];
    let r = at(rec("s", None, None), Some(T1));
    assert_eq!(
        with_caps(r, &caps, false),
        ("feat/cap".to_owned(), BranchBasis::Captured)
    );
}

#[test]
fn captured_beats_worktree_and_origin_recovery() {
    let caps = [
        cap("s", CaptureKind::SessionStart, "feat/cap", T0),
        cap("s", CaptureKind::Stop, "feat/cap", T2),
    ];
    let stamped = at(rec("s", Some("worktree-agent-abc"), None), Some(T1));
    assert_eq!(
        with_caps(stamped, &caps, false),
        ("feat/cap".to_owned(), BranchBasis::Captured)
    );
    let origin = at(
        rec("s", None, Some(Origin::Worktree("agent-abc".to_owned()))),
        Some(T1),
    );
    assert_eq!(
        with_caps(origin, &caps, false),
        ("feat/cap".to_owned(), BranchBasis::Captured)
    );
}

#[test]
fn a_real_stamp_is_never_replaced_by_a_capture() {
    let caps = [cap("s", CaptureKind::SessionStart, "feat/cap", T0)];
    let r = at(rec("s", Some("feat/stamped"), None), Some(T1));
    assert_eq!(
        with_caps(r, &caps, false),
        ("feat/stamped".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn strict_ignores_captures() {
    let caps = [cap("s", CaptureKind::SessionStart, "feat/cap", T0)];
    let r = at(rec("s", None, None), Some(T1));
    assert_eq!(
        with_caps(r, &caps, true),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn disagreeing_brackets_are_not_guessed() {
    let caps = [
        cap("s", CaptureKind::SessionStart, "main", T0),
        cap("s", CaptureKind::Stop, "feat/cap", T2),
    ];
    let r = at(rec("s", Some("worktree-agent-abc"), None), Some(T1));
    // Falls back to the existing worktree recovery.
    assert_eq!(
        with_caps(r, &caps, false),
        ("feat/real".to_owned(), BranchBasis::WorktreeRecovered)
    );
    let none = at(rec("s", None, None), Some(T1));
    assert_eq!(
        with_caps(none, &caps, false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn a_record_without_a_usable_timestamp_is_not_guessed() {
    let caps = [cap("s", CaptureKind::SessionStart, "feat/cap", T0)];
    for ts in [None, Some("not a time")] {
        let r = at(rec("s", None, None), ts);
        assert_eq!(
            with_caps(r, &caps, false),
            ("(none)".to_owned(), BranchBasis::Stamped)
        );
    }
}

#[test]
fn one_sided_captures_do_not_recover_a_record() {
    // SessionStart on feat/cap only: the user may have switched branches since, and nothing
    // after the record says otherwise. Not Captured; the existing recovery leaves it uncertain.
    let before_only = [cap("s", CaptureKind::SessionStart, "feat/cap", T0)];
    let r = at(rec("s", None, None), Some(T1));
    assert_eq!(
        with_caps(r, &before_only, false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
    // Captures only after the record, all agreeing: still one-sided.
    let early = at(rec("s", None, None), Some("2026-11-01T09:00:00Z"));
    let after_only = [
        cap("s", CaptureKind::SessionStart, "feat/cap", T1),
        cap("s", CaptureKind::Stop, "feat/cap", T2),
    ];
    assert_eq!(
        with_caps(early, &after_only, false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn captures_of_other_sessions_and_unusable_branches_are_ignored() {
    let other = [cap("other", CaptureKind::SessionStart, "feat/cap", T0)];
    let r = at(rec("s", None, None), Some(T1));
    assert_eq!(
        with_caps(r.clone(), &other, false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
    let detached = [cap("s", CaptureKind::SessionStart, "HEAD", T0)];
    assert_eq!(
        with_caps(r, &detached, false),
        ("(none)".to_owned(), BranchBasis::Stamped)
    );
}

#[test]
fn captured_branches_feed_the_claim() {
    let caps = [
        cap("s", CaptureKind::SessionStart, "feat/cap", T0),
        cap("s", CaptureKind::Stop, "feat/cap", T2),
    ];
    let rs = attribute_with_captures(
        [
            at(rec("s", None, None), Some(T1)),
            at(rec("s", Some("main"), None), Some(T1)),
        ],
        &[],
        &caps,
        &map(),
        false,
    );
    assert_eq!(select(rs, "feat/cap", false).len(), 2);
}

#[test]
fn subagent_rollup_is_kept_and_its_workspace_tag_needs_no_worktree_list() {
    // The worktree is absent from the map (removed), yet the tag comes from the cwd and the
    // record still rolls up to its dispatcher, with captures present.
    let caps = [cap("orch", CaptureKind::SessionStart, "feat/cap", T0)];
    let r = sub(
        "abc",
        Some("worktree-agent-gone"),
        Some(&format!(r"{WT_BACKSLASH}\agent-gone")),
    );
    let a = attribute_with_captures(
        [r],
        &[dispatch("abc", Some("feat/dispatcher"))],
        &caps,
        &WorktreeMap::default(),
        false,
    )
    .remove(0);
    assert_eq!(a.branch, "feat/dispatcher");
    assert_eq!(a.basis, BranchBasis::DispatcherRolledUp);
    let tags = a.subagent.expect("subagent tags");
    assert!(matches!(tags.workspace, Workspace::Worktree(n) if n == "agent-gone"));
}
