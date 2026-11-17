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
fn a_resolution_is_matched_to_the_pending_line_the_reader_keeps() {
    // Two conflicting pending lines for one key: the reader keeps the later `recorded_at`, which
    // here comes first in the file. A resolution copying that winner is faithful.
    let base = [
        entry("r1", "main")
            .pending()
            .output(5)
            .with("recorded_at", serde_json::json!("2026-01-03T00:00:00Z")),
        entry("r1", "main")
            .pending()
            .output(6)
            .with("recorded_at", serde_json::json!("2026-01-02T00:00:00Z")),
    ];
    let head = [
        entry("r1", "main")
            .pending()
            .output(5)
            .with("recorded_at", serde_json::json!("2026-01-03T00:00:00Z")),
        entry("r1", "main")
            .pending()
            .output(6)
            .with("recorded_at", serde_json::json!("2026-01-02T00:00:00Z")),
        entry("r1", "feat/x").resolves("main").output(5),
    ];
    assert_eq!(rules(&run(&base, &head)), Vec::<Rule>::new());
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

fn run_bytes(
    base: (Option<&[u8]>, Option<&[u8]>),
    head: (Option<&[u8]>, Option<&[u8]>),
    limit: Option<u64>,
) -> CheckReport {
    let input = CheckInput {
        base_ledger: base.0,
        head_ledger: head.0,
        base_captures: base.1,
        head_captures: head.1,
        head_branch: "feat/x",
        base_branch: "main",
        now: &now(),
    };
    match limit {
        Some(limit) => check_with_limit(&input, limit),
        None => check(&input),
    }
}

fn overlong() -> Vec<u8> {
    let mut v = vec![b'a'; MAX_LINE_BYTES + 1];
    v.push(b'\n');
    v
}

#[test]
fn an_overlong_new_ledger_line_is_a_violation_with_its_line_number() {
    let mut head = join(&[entry("r1", "feat/x")]);
    head.extend(overlong());
    let r = run_bytes((None, None), (Some(&head), None), None);
    assert_eq!(
        r.violations,
        vec![Violation {
            file: CheckedFile::Ledger,
            line: Some(2),
            rule: Rule::LineTooLong
        }]
    );
}

#[test]
fn an_overlong_new_captures_line_is_a_violation_with_its_line_number() {
    let mut head = b"\n".to_vec();
    head.extend(overlong());
    let r = run_bytes((None, None), (None, Some(&head)), None);
    assert_eq!(
        r.violations,
        vec![Violation {
            file: CheckedFile::Captures,
            line: Some(2),
            rule: Rule::LineTooLong
        }]
    );
}

#[test]
fn a_line_of_exactly_the_cap_is_parsed_not_flagged() {
    let mut head = vec![b'a'; MAX_LINE_BYTES];
    head.push(b'\n');
    let r = run_bytes((None, None), (Some(&head), None), None);
    assert_eq!(rules(&r), vec![Rule::Unparseable]);
}

#[test]
fn a_file_over_the_limit_fails_as_too_large_and_is_not_checked() {
    let head = join(&[entry("r1", "main")]);
    let limit = head.len() as u64 - 1;
    let r = run_bytes((None, None), (Some(&head), None), Some(limit));
    assert_eq!(
        r.violations,
        vec![Violation {
            file: CheckedFile::Ledger,
            line: None,
            rule: Rule::FileTooLarge
        }]
    );
    let r = run_bytes((None, None), (Some(&head), None), Some(head.len() as u64));
    assert_eq!(rules(&r), vec![Rule::RecordedForBaseBranch]);
}

#[test]
fn a_base_or_captures_file_over_the_limit_is_too_large_too() {
    let big = vec![b'\n'; 11];
    let r = run_bytes((Some(&big), None), (None, Some(&big)), Some(10));
    assert_eq!(
        r.violations,
        vec![
            Violation {
                file: CheckedFile::Ledger,
                line: None,
                rule: Rule::FileTooLarge
            },
            Violation {
                file: CheckedFile::Captures,
                line: None,
                rule: Rule::FileTooLarge
            },
        ]
    );
}

#[test]
fn check_uses_the_production_file_limit() {
    // A zero-filled allocation is lazily mapped, so this is cheap.
    let big = vec![0u8; usize::try_from(MAX_TALLY_FILE_BYTES).unwrap() + 1];
    let r = run_bytes((None, None), (Some(&big), None), None);
    assert_eq!(rules(&r), vec![Rule::FileTooLarge]);
}

fn line_too_long(file: CheckedFile, line: u64) -> Violation {
    Violation {
        file,
        line: Some(line),
        rule: Rule::LineTooLong,
    }
}

#[test]
fn a_line_of_exactly_the_cap_plus_a_cr_is_too_long_in_both_files() {
    // The reader counts the CR, so the check must measure the physical line.
    let mut head = vec![b'a'; MAX_LINE_BYTES];
    head.extend(b"\r\n");
    let r = run_bytes((None, None), (Some(&head), None), None);
    assert_eq!(r.violations, vec![line_too_long(CheckedFile::Ledger, 1)]);
    let r = run_bytes((None, None), (None, Some(&head)), None);
    assert_eq!(r.violations, vec![line_too_long(CheckedFile::Captures, 1)]);
}

#[test]
fn an_overlong_whitespace_only_line_is_too_long_in_both_files() {
    let mut head = b"\n".to_vec();
    head.extend(vec![b' '; MAX_LINE_BYTES + 1]);
    head.push(b'\n');
    let r = run_bytes((None, None), (Some(&head), None), None);
    assert_eq!(r.violations, vec![line_too_long(CheckedFile::Ledger, 2)]);
    let r = run_bytes((None, None), (None, Some(&head)), None);
    assert_eq!(r.violations, vec![line_too_long(CheckedFile::Captures, 2)]);
}

#[test]
fn a_whitespace_only_line_at_the_cap_is_still_ignored() {
    let mut head = vec![b' '; MAX_LINE_BYTES];
    head.push(b'\n');
    assert!(run_bytes((None, None), (Some(&head), None), None).is_clean());
}
