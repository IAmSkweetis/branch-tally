//! Integration tests for the ledger (#8): real git merges and the privacy guarantee.

use std::path::Path;
use std::process::Command;

use branch_tally_core::ledger::{self, Clock, Identity, LEDGER_PATH};
use branch_tally_core::model::{
    AgentId, AttributedRecord, Attribution, BranchBasis, Harness, Mode, Origin, Record, RequestId,
    SelectedRecord, Selection, SessionId, Usage,
};
use branch_tally_core::paths;

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
        "feat/x",
        Mode::Claim,
        None,
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
    ledger::append(
        &ledger_path,
        &a,
        "feat/x",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    git_ok(root, &["commit", "-q", "-am", "a"]);

    git_ok(root, &["checkout", "-q", "main"]);
    git_ok(root, &["checkout", "-q", "-b", "agent-b"]);
    let b = [
        selected("shared", "feat/x", 10),
        selected("only-b", "feat/x", 1000),
    ];
    ledger::append(
        &ledger_path,
        &b,
        "feat/x",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
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
    ledger::append(
        &p,
        &[selected("base", "feat/x", 1)],
        "feat/x",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "base"]);

    git_ok(root, &["checkout", "-q", "-b", "agent-a"]);
    let early = Fixed("2026-01-02T00:00:00Z");
    ledger::append(
        &p,
        &[selected("dup", "feat/x", 10)],
        "feat/x",
        Mode::Claim,
        None,
        &Identity,
        &early,
    )
    .unwrap();
    git_ok(root, &["commit", "-q", "-am", "a"]);

    git_ok(root, &["checkout", "-q", "main"]);
    git_ok(root, &["checkout", "-q", "-b", "agent-b"]);
    let late = Fixed("2026-01-03T00:00:00Z");
    ledger::append(
        &p,
        &[selected("dup", "feat/x", 10)],
        "feat/x",
        Mode::Claim,
        None,
        &Identity,
        &late,
    )
    .unwrap();
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
    ledger::append(&p, &[s], "feat/x", Mode::Claim, None, &Identity, &CLOCK).unwrap();
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
        "mode",
        "model",
        "origin",
        "recorded_at",
        "recorded_for",
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
        "source_cwd",
    ] {
        assert!(!keys.contains(&forbidden), "forbidden key {forbidden}");
    }
}

const MAIN: &str = "/home/alice-smith/workspaces/myrepo";

/// Appends one record whose session origin is the worktree slug of `cwd`, and returns the line
/// text and the append outcome.
#[expect(clippy::unwrap_used, reason = "test helper: fail loudly")]
fn append_worktree_origin(slug: &str) -> (String, ledger::AppendOutcome, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.keep().join(LEDGER_PATH);
    let mut s = selected("r1", "feat/x", 5);
    s.attributed.record.origin = Some(Origin::worktree(slug));
    s.attributed.record.cwd = Some(format!("{MAIN}/.claude/worktrees/agent-abc"));
    let out = ledger::append(
        &p,
        &[s],
        "feat/x",
        Mode::Claim,
        Some(Path::new(MAIN)),
        &Identity,
        &CLOCK,
    )
    .unwrap();
    (std::fs::read_to_string(&p).unwrap(), out, p)
}

#[test]
fn a_worktree_origin_is_stored_relative_to_the_repo_with_no_path_or_user() {
    let worktree = Path::new(MAIN)
        .join(".claude")
        .join("worktrees")
        .join("agent-abc");
    let slug = paths::slug(&worktree);
    let main_slug = paths::slug(Path::new(MAIN));
    assert!(
        slug.contains("alice"),
        "the realistic slug carries the user"
    );
    let (text, out, p) = append_worktree_origin(&slug);
    assert!(!text.contains(&main_slug), "main slug leaked: {text}");
    assert!(!text.contains("alice"), "user segment leaked: {text}");
    assert!(!text.contains("home"), "a path leaked: {text}");
    let line: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
    assert!(!line["origin"].to_string().contains('/'), "a path leaked");
    assert_eq!(
        line["origin"],
        serde_json::json!({"worktree": "claude-worktrees-agent-abc"})
    );
    assert_eq!(out.unnamed_origins, 0);
    // Read back on the same machine: the origin equals the live one exactly.
    let read = ledger::read(&p).unwrap();
    let back = read.entries()[0].to_selected(Some(Path::new(MAIN)));
    assert_eq!(back.attributed.record.origin, Some(Origin::worktree(slug)));
}

#[test]
fn the_main_slug_is_matched_case_insensitively() {
    let slug =
        paths::slug(&Path::new(MAIN).join(".claude").join("worktrees").join("w")).to_uppercase();
    let (text, out, _) = append_worktree_origin(&slug);
    assert!(
        text.contains(r#""origin":{"worktree":"CLAUDE-WORKTREES-W"}"#),
        "{text}"
    );
    assert_eq!(out.unnamed_origins, 0);
}

#[test]
fn a_slug_outside_the_main_checkout_is_unnamed_and_counted() {
    for outside in [
        paths::slug(Path::new("/home/bob/elsewhere/wt")),
        // Shares the main slug as a prefix but is a different directory.
        format!("{}2--claude-worktrees-a", paths::slug(Path::new(MAIN))),
        // The main checkout itself is not a worktree name.
        paths::slug(Path::new(MAIN)),
    ] {
        let (text, out, p) = append_worktree_origin(&outside);
        assert!(
            !text.contains("bob") && !text.contains("elsewhere"),
            "{text}"
        );
        assert!(text.contains(r#""origin":{"worktree":null}"#), "{text}");
        assert_eq!(out.unnamed_origins, 1, "{outside}");
        let read = ledger::read(&p).unwrap();
        let back = read.entries()[0].to_selected(Some(Path::new(MAIN)));
        assert_eq!(back.attributed.record.origin, None, "no name is claimed");
    }
}

#[test]
fn a_line_without_origin_still_reads_with_origin_none() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(LEDGER_PATH);
    let mut s = selected("r1", "feat/x", 5);
    s.attributed.record.origin = None;
    ledger::append(&p, &[s], "feat/x", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("origin"), "a None origin is omitted");
    let read = ledger::read(&p).unwrap();
    assert_eq!(
        read.entries()[0].to_selected(None).attributed.record.origin,
        None
    );
}
