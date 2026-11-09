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
    let a = attribute([r], &map(), strict).remove(0);
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
    let out = attribute(rs, &map(), false);
    let sessions: Vec<_> = out.iter().map(|a| a.record.session_id.as_str()).collect();
    assert_eq!(sessions, ["a", "b", "c"]);
}

fn attributed(rs: Vec<Record>) -> Vec<AttributedRecord> {
    attribute(rs, &WorktreeMap::default(), true)
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
        &map(),
        false,
    );
    let out = select(rs, "feat/real", false);
    assert_eq!(out.len(), 2);
}
