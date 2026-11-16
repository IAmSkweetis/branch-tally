use super::*;
use crate::ledger::SCHEMA_VERSION;

fn now() -> Timestamp {
    "2026-02-01T00:00:00Z".parse().unwrap()
}

struct Line {
    rid: &'static str,
    recorded_for: &'static str,
    pending: bool,
    resolved_from: Option<&'static str>,
    output: u64,
    extra: Vec<(&'static str, serde_json::Value)>,
}

fn entry(rid: &'static str, recorded_for: &'static str) -> Line {
    Line {
        rid,
        recorded_for,
        pending: false,
        resolved_from: None,
        output: 5,
        extra: Vec::new(),
    }
}

impl Line {
    fn pending(mut self) -> Self {
        self.pending = true;
        self
    }
    fn resolves(mut self, from: &'static str) -> Self {
        self.resolved_from = Some(from);
        self
    }
    fn output(mut self, n: u64) -> Self {
        self.output = n;
        self
    }
    fn with(mut self, key: &'static str, value: serde_json::Value) -> Self {
        self.extra.push((key, value));
        self
    }
    fn text(&self) -> String {
        let mut v = serde_json::json!({
            "v": SCHEMA_VERSION,
            "request_id": self.rid,
            "harness": "claude-code",
            "session_id": "s1",
            "ts": "2026-01-01T00:00:00Z",
            "model": "claude-x",
            "recorded_for": self.recorded_for,
            "mode": "claim",
            "branch": self.recorded_for,
            "basis": "stamped",
            "kept_by": "exact",
            "usage": {"input": 1, "cache_write_5m": 0, "cache_write_1h": 0, "cache_read": 0, "output": self.output},
            "recorded_at": "2026-01-02T00:00:00Z",
        });
        if self.pending {
            v["pending"] = true.into();
        }
        if let Some(from) = self.resolved_from {
            v["resolved"] = serde_json::json!({ "from": from });
        }
        for (key, value) in &self.extra {
            v[*key] = value.clone();
        }
        v.to_string()
    }
}

fn join(lines: &[Line]) -> Vec<u8> {
    let mut s = lines.iter().map(Line::text).collect::<Vec<_>>().join("\n");
    s.push('\n');
    s.into_bytes()
}

fn run(base: &[Line], head: &[Line]) -> CheckReport {
    let (b, h) = (join(base), join(head));
    check(&CheckInput {
        base_ledger: Some(&b),
        head_ledger: Some(&h),
        base_captures: None,
        head_captures: None,
        head_branch: "feat/x",
        base_branch: "main",
        now: &now(),
    })
}

fn rules(r: &CheckReport) -> Vec<Rule> {
    r.violations.iter().map(|v| v.rule).collect()
}

#[test]
fn a_new_pending_base_branch_line_is_a_counted_note_not_a_violation() {
    let r = run(
        &[],
        &[entry("r1", "main").pending(), entry("r2", "main").pending()],
    );
    assert!(r.is_clean(), "{:?}", r.violations);
    assert_eq!(r.pending, 2);
}

#[test]
fn a_new_unflagged_base_branch_line_still_fails() {
    let r = run(&[], &[entry("r1", "main")]);
    assert_eq!(rules(&r), vec![Rule::RecordedForBaseBranch]);
    assert_eq!(r.pending, 0);
}

#[test]
fn an_existing_format_ledger_checks_as_before() {
    // No line has `pending` or `resolved`: the same verdicts the check always gave.
    let base = [entry("b1", "feat/old")];
    let head = [
        entry("b1", "feat/old"),
        entry("n1", "feat/x"),
        entry("n2", "other"),
    ];
    let r = run(&base, &head);
    assert!(r.is_clean());
    assert_eq!(r.pending, 0);
    assert_eq!(r.other_branches, vec![("other".to_owned(), 1)]);
}

#[test]
fn a_resolution_with_its_pending_line_on_head_is_clean() {
    let r = run(
        &[],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/x").resolves("main"),
        ],
    );
    assert!(r.is_clean(), "{:?}", r.violations);
    assert_eq!(r.pending, 1);
}

#[test]
fn a_resolution_may_resolve_a_pending_line_already_on_base() {
    let r = run(
        &[entry("r1", "main").pending()],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/x").resolves("main"),
        ],
    );
    assert!(r.is_clean(), "{:?}", r.violations);
    assert_eq!(r.pending, 0, "the pending line is not new");
}

#[test]
fn a_line_that_is_both_pending_and_resolved_is_an_invalid_kind() {
    let r = run(
        &[],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/x").pending().resolves("main"),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::InvalidLineKind]);
}

#[test]
fn a_resolution_with_no_pending_line_is_an_orphan() {
    let r = run(&[], &[entry("r1", "feat/x").resolves("main")]);
    assert_eq!(rules(&r), vec![Rule::OrphanResolution]);
}

#[test]
fn a_resolution_whose_pending_line_is_for_another_origin_is_an_orphan() {
    let r = run(
        &[],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/x").resolves("master"),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::OrphanResolution]);
}

#[test]
fn a_resolution_that_changes_the_usage_is_a_mismatch() {
    let r = run(
        &[],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/x").resolves("main").output(6),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::ResolutionMismatch]);
}

#[test]
fn a_request_resolved_to_two_branches_conflicts() {
    let r = run(
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/y").resolves("main"),
        ],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/y").resolves("main"),
            entry("r1", "feat/x").resolves("main"),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::ConflictingResolution]);
}

#[test]
fn a_new_pending_line_not_recorded_for_the_base_branch_fails() {
    let r = run(&[], &[entry("r1", "feat/x").pending()]);
    assert_eq!(rules(&r), vec![Rule::PendingNotOnBase]);
}

#[test]
fn a_pending_line_for_another_branch_is_not_counted_as_a_pending_note() {
    let r = run(&[], &[entry("r1", "feat/y").pending()]);
    assert_eq!(r.pending, 0);
}

#[test]
fn a_resolution_that_changes_the_origin_is_a_mismatch() {
    let r = run(
        &[],
        &[
            entry("r1", "main")
                .pending()
                .with("origin", serde_json::json!("main")),
            entry("r1", "feat/x")
                .resolves("main")
                .with("origin", serde_json::json!({"worktree": "w"})),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::ResolutionMismatch]);
}

#[test]
fn a_resolution_whose_branch_differs_from_its_recorded_for_is_a_mismatch() {
    // A resolution names one destination (decision 0006): `branch` must equal `recorded_for`.
    let r = run(
        &[],
        &[
            entry("r1", "main").pending(),
            entry("r1", "feat/x")
                .resolves("main")
                .with("branch", serde_json::json!("feat/y")),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::ResolutionMismatch]);
}

#[test]
fn a_resolution_that_changes_the_subagent_or_agent_id_is_a_mismatch() {
    let tags =
        |name: &str| serde_json::json!({"workspace": "main", "subagent": name, "agent_id": "a1"});
    let r = run(
        &[],
        &[
            entry("r1", "main").pending().with("subagent", tags("one")),
            entry("r1", "feat/x")
                .resolves("main")
                .with("subagent", tags("two")),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::ResolutionMismatch]);
    let r = run(
        &[],
        &[
            entry("r1", "main").pending().with("agent_id", "a1".into()),
            entry("r1", "feat/x")
                .resolves("main")
                .with("agent_id", "a2".into()),
        ],
    );
    assert_eq!(rules(&r), vec![Rule::ResolutionMismatch]);
}

#[test]
fn a_faithful_resolution_of_a_fully_populated_pending_line_is_clean() {
    let tags = serde_json::json!({"workspace": "main", "subagent": "one", "agent_id": "a1"});
    let r = run(
        &[],
        &[
            entry("r1", "main")
                .pending()
                .with("origin", serde_json::json!("main"))
                .with("subagent", tags.clone())
                .with("agent_id", "a1".into()),
            entry("r1", "feat/x")
                .resolves("main")
                .with("origin", serde_json::json!("main"))
                .with("subagent", tags)
                .with("agent_id", "a1".into()),
        ],
    );
    assert!(r.is_clean(), "{:?}", r.violations);
}
