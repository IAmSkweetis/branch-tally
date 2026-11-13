//! End-to-end tests for `ledger check --base <ref>` (#96): the committed `.tally/` files on the
//! head may only append well-formed lines to the base's. Repositories and lines are synthetic.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup fails loudly"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_branch-tally");
const BRANCH: &str = "feat/x";
/// Marks text that must never reach stderr.
const SECRET: &str = "req_SECRET_ID_0001";

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=bt",
            "-c",
            "user.email=bt@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("these tests require `git` on PATH");
    assert!(out.status.success(), "git {args:?} failed");
}

struct Repo {
    _scratch: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn file(&self, name: &str) -> PathBuf {
        self.root.join(".tally").join(name)
    }

    fn write(&self, name: &str, text: &str) {
        fs::create_dir_all(self.root.join(".tally")).unwrap();
        fs::write(self.file(name), text).unwrap();
    }

    fn append(&self, name: &str, text: &str) {
        let mut old = fs::read_to_string(self.file(name)).unwrap_or_default();
        old.push_str(text);
        self.write(name, &old);
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .current_dir(&self.root)
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("run branch-tally")
    }

    fn check(&self) -> Output {
        self.run(&["ledger", "check", "--base", "main", "--head-branch", BRANCH])
    }
}

fn ledger_line(request: &str, branch: &str, recorded_at: Option<&str>, output: u64) -> String {
    let mut v = json!({
        "v": 1, "request_id": request, "harness": "claude-code", "session_id": "s-1",
        "ts": "2026-01-01T00:00:00Z", "model": "claude-sonnet-4-5", "recorded_for": branch,
        "mode": "claim", "branch": branch, "basis": "stamped", "kept_by": "session-claim",
        "origin": "main",
        "usage": {"input": 1, "cache_write_5m": 0, "cache_write_1h": 0, "cache_read": 0,
                  "output": output, "thinking": 0},
    });
    if let Some(at) = recorded_at {
        v["recorded_at"] = Value::String(at.to_owned());
    }
    format!("{v}\n")
}

fn good(request: &str) -> String {
    ledger_line(request, BRANCH, Some("2026-01-02T00:00:00Z"), 5)
}

fn capture_line(ts: &str) -> String {
    format!(
        "{}\n",
        json!({"v": 1, "harness": "claude-code", "kind": "stop", "session_id": "s-1",
               "branch": BRANCH, "ts": ts})
    )
}

fn bare_repo() -> Repo {
    let scratch = tempfile::Builder::new()
        .prefix("bt-lcheck-")
        .tempdir()
        .unwrap();
    let root = branch_tally_core::git::normalize_path(scratch.path()).join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    Repo {
        _scratch: scratch,
        root,
    }
}

/// A repository whose `main` holds one ledger line and one capture, checked out on `feat/x`; the
/// head is the working tree.
fn repo() -> Repo {
    let r = bare_repo();
    r.write(
        "ledger.jsonl",
        &ledger_line("req_base_1", "main", Some("2026-01-01T00:00:00Z"), 5),
    );
    r.write("captures.jsonl", &capture_line("2026-01-01T00:00:00Z"));
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "base"]);
    git(&r.root, &["checkout", "-q", "-b", BRANCH]);
    r
}

fn err(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

fn assert_violation(o: &Output, needle: &[&str]) {
    let e = err(o);
    assert_eq!(o.status.code(), Some(1), "{e}");
    assert!(
        e.lines().any(|l| needle.iter().all(|n| l.contains(n))),
        "no line with {needle:?} in: {e}"
    );
    assert!(!e.contains(SECRET), "{e}");
    assert_eq!(o.stdout, b"");
}

#[test]
fn clean_append_exits_0_with_a_summary() {
    let r = repo();
    r.append("ledger.jsonl", &good("req_new_1"));
    r.append("captures.jsonl", &capture_line("2026-01-02T00:00:00Z"));
    let o = r.check();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(err(&o).lines().count(), 1, "{}", err(&o));
}

#[test]
fn unchanged_head_is_clean() {
    let r = repo();
    assert_eq!(r.check().status.code(), Some(0));
}

#[test]
fn head_branch_defaults_to_the_current_branch() {
    let r = repo();
    r.append("ledger.jsonl", &good("req_new_1"));
    let o = r.run(&["ledger", "check", "--base", "main"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
}

#[test]
fn removed_base_line() {
    let r = repo();
    r.write("ledger.jsonl", "");
    let o = r.check();
    assert_violation(&o, &["ledger", "removed base line"]);
    assert!(err(&o).contains("1 violation"), "{}", err(&o));
}

#[test]
fn rewritten_base_line_is_a_removal() {
    let r = repo();
    r.write(
        "ledger.jsonl",
        &ledger_line("req_base_1", "main", Some("2026-01-01T00:00:00Z"), 6),
    );
    assert_violation(&r.check(), &["ledger", "removed base line"]);
}

#[test]
fn unparseable_line_names_its_line_number() {
    let r = repo();
    r.append("ledger.jsonl", &format!("not json {SECRET}\n"));
    assert_violation(&r.check(), &["ledger", "line 2", "unparseable"]);
}

#[test]
fn wrong_branch() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(SECRET, "other", Some("2026-01-02T00:00:00Z"), 5),
    );
    assert_violation(&r.check(), &["ledger", "line 2", "wrong branch"]);
}

#[test]
fn missing_recorded_at() {
    let r = repo();
    r.append("ledger.jsonl", &ledger_line(SECRET, BRANCH, None, 5));
    assert_violation(&r.check(), &["ledger", "line 2", "missing recorded_at"]);
}

#[test]
fn invalid_timestamp() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(SECRET, BRANCH, Some("yesterday"), 5),
    );
    assert_violation(&r.check(), &["ledger", "line 2", "invalid timestamp"]);
}

#[test]
fn future_timestamp() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(SECRET, BRANCH, Some("2999-01-01T00:00:00Z"), 5),
    );
    assert_violation(&r.check(), &["ledger", "line 2", "future timestamp"]);
}

#[test]
fn duplicate_key() {
    let r = repo();
    // Same (request_id, recorded_for) as the base line, different usage.
    r.append(
        "ledger.jsonl",
        &ledger_line("req_base_1", "main", Some("2026-01-02T00:00:00Z"), 9),
    );
    let o = r.run(&["ledger", "check", "--base", "main", "--head-branch", "main"]);
    assert_violation(&o, &["ledger", "line 2", "duplicate"]);
}

#[test]
fn token_over_ceiling_names_the_field() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(SECRET, BRANCH, Some("2026-01-02T00:00:00Z"), 10_000_001),
    );
    assert_violation(&r.check(), &["ledger", "line 2", "output", "10000000"]);
}

#[test]
fn ceiling_itself_is_allowed() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(
            "req_new_1",
            BRANCH,
            Some("2026-01-02T00:00:00Z"),
            10_000_000,
        ),
    );
    assert_eq!(r.check().status.code(), Some(0));
}

#[test]
fn captures_violations_are_named_by_file() {
    let r = repo();
    r.append("captures.jsonl", "garbage\n");
    r.append("captures.jsonl", &capture_line("2999-01-01T00:00:00Z"));
    let o = r.check();
    assert_violation(&o, &["captures", "line 2", "unparseable"]);
    assert_violation(&o, &["captures", "line 3", "future timestamp"]);
    assert!(err(&o).contains("2 violation"), "{}", err(&o));
}

#[test]
fn removed_capture_line() {
    let r = repo();
    r.write("captures.jsonl", "");
    assert_violation(&r.check(), &["captures", "removed base line"]);
}

#[test]
fn missing_tally_on_both_sides_is_clean() {
    let r = bare_repo();
    git(&r.root, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let o = r.run(&["ledger", "check", "--base", "main", "--head-branch", "x"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
}

#[test]
fn tally_deleted_on_head_reports_removals() {
    let r = repo();
    fs::remove_dir_all(r.root.join(".tally")).unwrap();
    assert_violation(&r.check(), &["ledger", "removed base line"]);
}

#[test]
fn base_without_tally_checks_every_head_line() {
    let r = repo();
    git(&r.root, &["checkout", "-q", "-b", "empty-base", "main"]);
    git(&r.root, &["rm", "-q", "-r", ".tally"]);
    git(&r.root, &["commit", "-q", "-m", "drop"]);
    git(&r.root, &["checkout", "-q", BRANCH]);
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "empty-base",
        "--head-branch",
        BRANCH,
    ]);
    // The base holds nothing, so the head's older "main" line is new and on the wrong branch.
    assert_violation(&o, &["ledger", "line 1", "wrong branch"]);
}

#[test]
fn unknown_base_is_an_error_naming_the_ref() {
    let r = repo();
    let o = r.run(&["ledger", "check", "--base", "no-such-ref-zzz"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("no-such-ref-zzz"), "{}", err(&o));
}

#[test]
fn base_is_required() {
    let r = repo();
    assert_eq!(r.run(&["ledger", "check"]).status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn a_symlinked_head_file_is_refused() {
    let r = repo();
    fs::remove_file(r.file("ledger.jsonl")).unwrap();
    std::os::unix::fs::symlink(r.file("captures.jsonl"), r.file("ledger.jsonl")).unwrap();
    let o = r.check();
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
}
