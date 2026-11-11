//! Integration tests for the ledger (#8): real git merges and the privacy guarantee.

use std::path::Path;
use std::process::Command;

use branch_tally_core::ledger::{self, Clock, Identity, LEDGER_PATH};
use branch_tally_core::model::{
    AgentId, AttributedRecord, Attribution, BranchBasis, Harness, Record, RequestId,
    SelectedRecord, Selection, SessionId, Usage,
};

fn selected(id: &str, branch: &str, output: u64) -> SelectedRecord {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new("sess"),
        RequestId::new(id),
        Usage {
            output,
            ..Usage::default()
        },
    );
    r.model = Some("claude-x".to_owned());
    r.ts = Some("2026-01-01T00:00:00Z".to_owned());
    SelectedRecord {
        attributed: AttributedRecord {
            record: r,
            branch: branch.to_owned(),
            basis: BranchBasis::Stamped,
            subagent: None,
        },
        kept_by: Selection::Exact,
    }
}

/// Runs git in `dir` and returns the exit success and combined output.
#[expect(
    clippy::expect_used,
    reason = "a missing git must fail the test loudly, not skip it"
)]
fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .expect("git must be installed to run the ledger merge test");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let (ok, text) = git(dir, args);
    assert!(ok, "git {args:?} failed: {text}");
    text
}

#[test]
fn two_worktrees_record_concurrently_and_merge_without_conflicts_or_double_counting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q", "-b", "main"]);
    std::fs::write(
        root.join(".gitattributes"),
        format!("{LEDGER_PATH} merge=union\n"),
    )
    .unwrap();
    // A shared base record so both branches append to a non-empty file.
    let ledger_path = root.join(LEDGER_PATH);
    ledger::append(
        &ledger_path,
        &[selected("base", "feat/x", 1)],
        &Identity,
        &CLOCK,
    )
    .unwrap();
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "base"]);

    git_ok(root, &["checkout", "-q", "-b", "agent-a"]);
    let a = [
        selected("shared", "feat/x", 10),
        selected("only-a", "feat/x", 100),
    ];
    ledger::append(&ledger_path, &a, &Identity, &CLOCK).unwrap();
    git_ok(root, &["commit", "-q", "-am", "a"]);

    git_ok(root, &["checkout", "-q", "main"]);
    git_ok(root, &["checkout", "-q", "-b", "agent-b"]);
    let b = [
        selected("shared", "feat/x", 10),
        selected("only-b", "feat/x", 1000),
    ];
    ledger::append(&ledger_path, &b, &Identity, &CLOCK).unwrap();
    git_ok(root, &["commit", "-q", "-am", "b"]);

    git_ok(root, &["checkout", "-q", "main"]);
    git_ok(root, &["checkout", "-q", "-b", "feat/x"]);
    git_ok(root, &["merge", "-q", "--no-edit", "agent-a"]);
    let (ok, text) = git(root, &["merge", "--no-edit", "agent-b"]);
    assert!(ok, "merge conflicted: {text}");

    let merged = ledger::read(&ledger_path).unwrap();
    let mut ids: Vec<&str> = merged
        .entries()
        .iter()
        .map(|e| e.request_id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["base", "only-a", "only-b", "shared"]);
    let total: u64 = merged.entries().iter().map(|e| e.usage.output).sum();
    assert_eq!(total, 1 + 10 + 100 + 1000);
    let stats = merged.stats();
    assert_eq!(stats.malformed, 0);
    assert_eq!(stats.conflicts, 0);
    // Both branches wrote a byte-identical `shared` line (same fixed clock), and git's merge
    // keeps one copy of an identical added line, so the reader sees no duplicate.
    assert_eq!(stats.duplicates, 0, "{stats:?}");
}

struct Fixed(&'static str);
impl Clock for Fixed {
    fn now(&self) -> String {
        self.0.to_owned()
    }
}

const CLOCK: Fixed = Fixed("2026-01-01T00:00:00Z");

#[test]
fn the_same_id_recorded_at_different_times_survives_the_merge_as_one_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q", "-b", "main"]);
    std::fs::write(
        root.join(".gitattributes"),
        format!("{LEDGER_PATH} merge=union\n"),
    )
    .unwrap();
    let p = root.join(LEDGER_PATH);
    ledger::append(&p, &[selected("base", "feat/x", 1)], &Identity, &CLOCK).unwrap();
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "base"]);

    git_ok(root, &["checkout", "-q", "-b", "agent-a"]);
    let early = Fixed("2026-01-02T00:00:00Z");
    ledger::append(&p, &[selected("dup", "feat/x", 10)], &Identity, &early).unwrap();
    git_ok(root, &["commit", "-q", "-am", "a"]);

    git_ok(root, &["checkout", "-q", "main"]);
    git_ok(root, &["checkout", "-q", "-b", "agent-b"]);
    let late = Fixed("2026-01-03T00:00:00Z");
    ledger::append(&p, &[selected("dup", "feat/x", 10)], &Identity, &late).unwrap();
    git_ok(root, &["commit", "-q", "-am", "b"]);

    git_ok(root, &["checkout", "-q", "main"]);
    git_ok(root, &["checkout", "-q", "-b", "feat/x"]);
    git_ok(root, &["merge", "-q", "--no-edit", "agent-a"]);
    let (ok, text) = git(root, &["merge", "--no-edit", "agent-b"]);
    assert!(ok, "merge conflicted: {text}");

    let merged = ledger::read(&p).unwrap();
    let stats = merged.stats();
    assert_eq!(
        (
            stats.lines,
            stats.entries,
            stats.duplicates,
            stats.conflicts
        ),
        (3, 2, 1, 0),
        "{stats:?}"
    );
    let total: u64 = merged.entries().iter().map(|e| e.usage.output).sum();
    assert_eq!(total, 11);
}

#[test]
fn a_written_line_carries_no_cwd_content_or_raw_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(LEDGER_PATH);
    let mut s = selected("r1", "feat/x", 5);
    {
        let r = &mut s.attributed.record;
        r.cwd = Some("/home/secret-user/project-cwd".to_owned());
        r.git_branch = Some("raw-stamp-branch".to_owned());
        r.agent_id = Some(AgentId::new("a1"));
        r.attribution = Attribution {
            skill: Some("sk".to_owned()),
            agent: None,
        };
    }
    ledger::append(&p, &[s], &Identity, &CLOCK).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("secret-user"), "cwd leaked");
    assert!(
        !text.contains("raw-stamp-branch"),
        "raw branch stamp leaked"
    );
    let line: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
    let mut keys: Vec<&str> = line
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let allowed = [
        "agent",
        "agent_id",
        "basis",
        "branch",
        "harness",
        "kept_by",
        "model",
        "recorded_at",
        "request_id",
        "session_id",
        "skill",
        "subagent",
        "ts",
        "usage",
        "v",
    ];
    for k in &keys {
        assert!(allowed.contains(k), "unexpected ledger key {k}");
    }
    for forbidden in [
        "cwd",
        "content",
        "message",
        "text",
        "prompt",
        "git_branch",
        "origin",
    ] {
        assert!(!keys.contains(&forbidden), "forbidden key {forbidden}");
    }
}
