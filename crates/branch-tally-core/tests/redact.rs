//! Integration tests for the redaction stage. All names and ids are synthetic.

#![expect(
    clippy::expect_used,
    reason = "a failed serialisation in a test helper should fail loudly"
)]

use std::collections::BTreeSet;

use branch_tally_core::model::{
    AgentId, Bucket, Mode, SessionId, SessionRow, SubagentRow, SubagentSummary, Summary, Workspace,
};
use branch_tally_core::redact::{RedactOptions, redact};

fn bucket(n: u64) -> Bucket {
    Bucket {
        input: n,
        output: n,
        calls: 1,
        ..Bucket::default()
    }
}

fn session(first: &str, last: &str, origin: &str, agents: &[&str]) -> (SessionId, SessionRow) {
    let mut row = SessionRow::new(first, origin);
    last.clone_into(&mut row.last);
    row.tokens = 10;
    row.calls = 2;
    row.agents = agents.iter().map(|a| (*a).to_owned()).collect();
    (SessionId::new(format!("real-{first}-{origin}")), row)
}

fn sub(ws: Workspace, name: &str, ids: &[&str]) -> SubagentRow {
    SubagentRow {
        workspace: ws,
        subagent: name.to_owned(),
        model: "m1".to_owned(),
        calls: 3,
        tokens: 30,
        agent_ids: ids.iter().map(|i| AgentId::new(*i)).collect(),
        by_model: vec![("m1".to_owned(), bucket(5))],
    }
}

fn sample() -> Summary {
    Summary {
        branch: "feat/x".to_owned(),
        mode: Mode::Claim,
        total: bucket(100),
        by_model: vec![("m1".to_owned(), bucket(60)), ("m2".to_owned(), bucket(40))],
        by_attribution: vec![
            ("main".to_owned(), bucket(1)),
            ("agent:zed".to_owned(), bucket(2)),
            ("deploy-skill".to_owned(), bucket(3)),
            ("agent:alpha".to_owned(), bucket(4)),
            ("other-skill".to_owned(), bucket(5)),
        ],
        sessions: vec![
            session(
                "2026-03-02T10:11:12Z",
                "2026-03-02T11:00:00Z",
                "main",
                &["zed"],
            ),
            session(
                "2026-03-01T09:00:00Z",
                "2026-03-03T01:02:03Z",
                "wt-secret",
                &["alpha", "zed"],
            ),
            session("", "", "main", &[]),
        ],
        subagents: SubagentSummary {
            rows: vec![
                sub(Workspace::worktree("WT-One"), "zed", &["id-a", "id-b"]),
                sub(Workspace::worktree("wt-one"), "alpha", &["id-b", "id-c"]),
                sub(Workspace::Main, "plain", &["id-d"]),
                sub(Workspace::Unknown, "agent-xyz", &["id-e"]),
            ],
            unattributed_calls: 4,
            unattributed_tokens: 40,
            unlinked_in_total_calls: 1,
            unlinked_in_total_tokens: 9,
        },
        ..Summary::default()
    }
}

fn all_text(s: &Summary) -> String {
    serde_json::to_string(s).expect("serialize")
}

#[test]
fn sets_flag_and_leaves_every_number_alone() {
    let before = sample();
    let after = redact(before.clone(), RedactOptions::default());
    assert!(after.redacted);
    assert_eq!(after.branch, before.branch);
    assert_eq!(after.mode, before.mode);
    assert_eq!(after.total, before.total);
    assert_eq!(after.footnote, before.footnote);
    assert_eq!(after.by_model, before.by_model);
    assert_eq!(after.work_branch, before.work_branch);
    let buckets = |s: &Summary| s.by_attribution.iter().map(|(_, b)| *b).collect::<Vec<_>>();
    assert_eq!(buckets(&after), buckets(&before));
    let (a, b) = (&after.subagents, &before.subagents);
    assert_eq!(
        (
            a.unattributed_calls,
            a.unattributed_tokens,
            a.unlinked_in_total_calls,
            a.unlinked_in_total_tokens
        ),
        (4, 40, 1, 9)
    );
    assert_eq!(a.rows.len(), b.rows.len());
    for (x, y) in a.rows.iter().zip(&b.rows) {
        assert_eq!(
            (x.calls, x.tokens, &x.model, &x.by_model),
            (y.calls, y.tokens, &y.model, &y.by_model)
        );
        assert_eq!(x.agent_ids.len(), y.agent_ids.len());
        assert_eq!(x.workspace.kind(), y.workspace.kind());
    }
    assert_eq!(a.dispatched(), b.dispatched());
    assert_eq!(a.workspaces(), b.workspaces());
    assert_eq!(a.worktrees(), b.worktrees());
}

#[test]
fn sessions_are_ordinal_in_original_first_order_with_dates_only() {
    let after = redact(sample(), RedactOptions::default());
    let got: Vec<_> = after
        .sessions
        .iter()
        .map(|(id, r)| {
            (
                id.as_str().to_owned(),
                r.first.clone(),
                r.last.clone(),
                r.origin.clone(),
            )
        })
        .collect();
    let t = |a: &str, b: &str, c: &str, d: &str| {
        (a.to_owned(), b.to_owned(), c.to_owned(), d.to_owned())
    };
    assert_eq!(
        got,
        vec![
            t("s1", "", "", "main"),
            t("s2", "2026-03-01", "2026-03-03", "worktree"),
            t("s3", "2026-03-02", "2026-03-02", "main"),
        ]
    );
    // tokens and calls travel with the row
    assert!(
        after
            .sessions
            .iter()
            .all(|(_, r)| (r.tokens, r.calls) == (10, 2))
    );
}

#[test]
fn equal_first_timestamps_keep_original_order() {
    let mut s = sample();
    s.sessions = vec![
        session("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z", "main", &[]),
        session("2026-01-01T00:00:00Z", "2026-01-01T05:00:00Z", "other", &[]),
    ];
    let after = redact(s, RedactOptions::default());
    assert_eq!(after.sessions[0].1.origin, "main");
    assert_eq!(after.sessions[1].1.origin, "worktree");
}

#[test]
fn malformed_timestamps_never_leak_a_time() {
    let mut s = sample();
    s.sessions = vec![
        session("2026-01-01", "short", "main", &[]),
        session("12:34:56 is a time", "2026-01-0", "main", &[]),
    ];
    let after = redact(s, RedactOptions::default());
    let firsts: Vec<_> = after
        .sessions
        .iter()
        .map(|(_, r)| r.first.as_str())
        .collect();
    assert!(firsts.contains(&"2026-01-01") && firsts.contains(&""));
    for (_, r) in &after.sessions {
        assert_eq!(r.last, "");
    }
    assert!(!all_text(&after).contains("12:34"));
}

#[test]
fn names_become_consistent_generic_labels() {
    let after = redact(sample(), RedactOptions::default());
    let attrs: Vec<_> = after
        .by_attribution
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(attrs[0], "main");
    assert!(attrs[1].starts_with("agent:agent-"));
    assert!(attrs[2].starts_with("skill-"));
    assert!(attrs[3].starts_with("agent:agent-"));
    assert!(attrs[4].starts_with("skill-"));
    let distinct: BTreeSet<_> = attrs.iter().collect();
    assert_eq!(distinct.len(), attrs.len());

    let zed = attrs[1].strip_prefix("agent:").expect("prefix");
    let alpha = attrs[3].strip_prefix("agent:").expect("prefix");
    assert_ne!(zed, alpha);
    // sessions' agents and subagent rows use the same labels
    let session_with_both = after
        .sessions
        .iter()
        .find(|(_, r)| r.agents.len() == 2)
        .expect("session with two agents");
    assert_eq!(
        session_with_both.1.agents,
        BTreeSet::from([alpha.to_owned(), zed.to_owned()])
    );
    assert_eq!(after.subagents.rows[0].subagent, zed);
    assert_eq!(after.subagents.rows[1].subagent, alpha);
    let text = all_text(&after);
    for real in [
        "zed",
        "alpha",
        "deploy-skill",
        "other-skill",
        "plain",
        "agent-xyz",
    ] {
        assert!(!text.contains(&format!("\"{real}\"")), "{real} leaked");
    }
}

#[test]
fn label_numbering_is_deterministic() {
    let a = redact(sample(), RedactOptions::default());
    let b = redact(sample(), RedactOptions::default());
    assert_eq!(a, b);
    // First seen in by_attribution order: zed is agent-1, alpha agent-2; skills likewise.
    assert_eq!(a.by_attribution[1].0, "agent:agent-1");
    assert_eq!(a.by_attribution[3].0, "agent:agent-2");
    assert_eq!(a.by_attribution[2].0, "skill-1");
    assert_eq!(a.by_attribution[4].0, "skill-2");
}

#[test]
fn names_false_keeps_names_but_still_redacts_ids_and_worktrees() {
    let after = redact(sample(), RedactOptions { names: false });
    assert!(after.redacted);
    let attrs: Vec<_> = after
        .by_attribution
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(
        attrs,
        [
            "main",
            "agent:zed",
            "deploy-skill",
            "agent:alpha",
            "other-skill"
        ]
    );
    assert_eq!(after.subagents.rows[0].subagent, "zed");
    let text = all_text(&after);
    for secret in [
        "real-",
        "wt-secret",
        "WT-One",
        "wt-one",
        "id-a",
        "id-b",
        "id-c",
        "id-d",
        "id-e",
    ] {
        assert!(!text.contains(secret), "{secret} leaked");
    }
}

#[test]
fn worktrees_get_ordinals_case_insensitively_and_kinds_survive() {
    let after = redact(sample(), RedactOptions::default());
    let rows = &after.subagents.rows;
    assert_eq!(rows[0].workspace, rows[1].workspace);
    assert_eq!(rows[0].workspace.label(), "worktree-1");
    assert!(rows[0].workspace.is_worktree());
    assert_eq!(rows[2].workspace, Workspace::Main);
    assert_eq!(rows[3].workspace, Workspace::Unknown);
}

#[test]
fn agent_ids_are_synthetic_distinct_and_consistent_across_rows() {
    let after = redact(sample(), RedactOptions::default());
    let rows = &after.subagents.rows;
    let all: BTreeSet<_> = rows
        .iter()
        .flat_map(|r| r.agent_ids.iter().cloned())
        .collect();
    assert_eq!(all.len(), 5);
    // id-b appears in rows 0 and 1, so exactly one id is shared.
    let shared: Vec<_> = rows[0].agent_ids.intersection(&rows[1].agent_ids).collect();
    assert_eq!(shared.len(), 1);
}
