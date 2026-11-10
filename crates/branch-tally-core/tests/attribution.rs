//! Attribution: worktree recovery and record selection, ported from `reference/token_report.py`.

use std::path::Path;

use branch_tally_core::attribution::{WorktreeMap, attribute, select, slug_for};
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

#[test]
fn slug_replaces_every_non_alphanumeric() {
    assert_eq!(slug_for(Path::new(r"D:\work\Repo")), "D--work-Repo");
    assert_eq!(slug_for(Path::new("/home/me/my.repo")), "-home-me-my-repo");
    assert_eq!(slug_for(Path::new("D:/work/Repo")), "D--work-Repo");
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
fn main_origin_none_looks_up_the_main_checkout_slug_like_the_reference() {
    // Reference: `recovered = wt_branches.get(origin_name)` also runs for the main root, and the
    // main checkout is in the worktree list. Ported as-is.
    let (b, basis) = one(rec("s", None, Some(Origin::Main)), false);
    assert_eq!((b.as_str(), basis), ("main", BranchBasis::OriginRecovered));
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
