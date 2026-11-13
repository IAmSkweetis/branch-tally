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
        self.run(&[
            "ledger",
            "check",
            "--base",
            "main",
            "--base-branch",
            "main",
            "--head-branch",
            BRANCH,
        ])
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
    let o = r.run(&["ledger", "check", "--base", "main", "--base-branch", "main"]);
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
fn recorded_for_base_branch() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(SECRET, "main", Some("2026-01-02T00:00:00Z"), 5),
    );
    assert_violation(
        &r.check(),
        &["ledger", "line 2", "recorded for the base branch"],
    );
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
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "main",
        "--base-branch",
        "main",
        "--head-branch",
        "main",
    ]);
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
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "main",
        "--base-branch",
        "main",
        "--head-branch",
        "x",
    ]);
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
    // The base branch is the repository's default branch, so give it an `origin/HEAD`.
    git(&r.root, &["update-ref", "refs/remotes/origin/main", "main"]);
    git(
        &r.root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "empty-base",
        "--head-branch",
        BRANCH,
    ]);
    // The base holds nothing, so the head's older "main" line is new and recorded for the base branch.
    assert_violation(&o, &["ledger", "line 1", "recorded for the base branch"]);
}

#[test]
fn unknown_base_is_an_error_naming_the_ref() {
    let r = repo();
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "no-such-ref-zzz",
        "--base-branch",
        "main",
    ]);
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

// ---- #96: --head, --base-branch, --head-branch, notes and warnings ----

fn head_sha(r: &Repo) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&r.root)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn commit_all(r: &Repo, msg: &str) {
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", msg]);
}

#[test]
fn head_rev_is_read_from_git_not_the_working_tree() {
    let r = repo();
    r.append("ledger.jsonl", &good("req_new_1"));
    commit_all(&r, "head");
    // The working tree now breaks the rule; the committed head does not.
    r.write("ledger.jsonl", "");
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "main",
        "--base-branch",
        "main",
        "--head",
        "HEAD",
        "--head-branch",
        BRANCH,
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    // Without --head the same tree is read and fails.
    assert_eq!(r.check().status.code(), Some(1));
}

#[test]
fn head_rev_violation_is_found_even_when_the_working_tree_is_clean() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line("req_new_1", "main", Some("2026-01-02T00:00:00Z"), 5),
    );
    commit_all(&r, "head");
    git(&r.root, &["checkout", "-q", "main", "--", ".tally"]);
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "main",
        "--base-branch",
        "main",
        "--head",
        "HEAD",
        "--head-branch",
        BRANCH,
    ]);
    assert_violation(&o, &["ledger", "line 2", "recorded for the base branch"]);
}

#[test]
fn unknown_head_rev_exits_1_naming_it() {
    let r = repo();
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "main",
        "--base-branch",
        "main",
        "--head",
        "no-such-rev-xyz",
        "--head-branch",
        BRANCH,
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("no-such-rev-xyz"), "{}", err(&o));
}

#[test]
fn missing_base_branch_with_no_origin_head_exits_1_and_says_to_pass_it() {
    let r = repo();
    // Even though --base is a branch name, it is never used as the base branch.
    let o = r.run(&["ledger", "check", "--base", "main", "--head-branch", BRANCH]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("--base-branch"), "{}", err(&o));
}

#[test]
fn base_branch_flag_makes_the_base_rule_fire_with_a_sha_base() {
    let r = repo();
    let sha = head_sha(&r);
    r.append(
        "ledger.jsonl",
        &ledger_line("req_new_1", "main", Some("2026-01-02T00:00:00Z"), 5),
    );
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        &sha,
        "--base-branch",
        "main",
        "--head-branch",
        BRANCH,
    ]);
    assert_violation(&o, &["ledger", "line 2", "recorded for the base branch"]);
}

#[test]
fn detached_head_without_head_branch_exits_1_and_says_to_pass_it() {
    let r = repo();
    r.append("ledger.jsonl", &good("req_new_1"));
    git(&r.root, &["checkout", "-q", "--detach"]);
    let o = r.run(&["ledger", "check", "--base", "main", "--base-branch", "main"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("--head-branch"), "{}", err(&o));
    // Passing it works.
    let o = r.check();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
}

#[test]
fn other_branch_lines_are_noted_without_changing_the_exit_code() {
    let r = repo();
    r.append("ledger.jsonl", &good("req_new_1"));
    r.append(
        "ledger.jsonl",
        &ledger_line("req_other_1", "feat/other", Some("2026-01-02T00:00:00Z"), 5),
    );
    r.append(
        "ledger.jsonl",
        &ledger_line("req_other_2", "feat/other", Some("2026-01-02T00:00:00Z"), 5),
    );
    let o = r.check();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(
        err(&o)
            .lines()
            .any(|l| l == "note: 2 new ledger line(s) recorded for branch feat/other"),
        "{}",
        err(&o)
    );
}

#[test]
fn hostile_branch_names_are_neutralised_in_notes() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line(
            "req_o1",
            "evil\n::error::boom\u{1b}[31m",
            Some("2026-01-02T00:00:00Z"),
            5,
        ),
    );
    r.append(
        "ledger.jsonl",
        &ledger_line("req_o2", "::warning::x", Some("2026-01-02T00:00:00Z"), 5),
    );
    let o = r.check();
    let e = err(&o);
    assert_eq!(o.status.code(), Some(0), "{e}");
    assert!(!e.contains('\u{1b}'), "{e:?}");
    assert!(e.lines().all(|l| !l.starts_with("::")), "{e:?}");
    assert!(
        e.lines().all(|l| !l.trim_start().starts_with("::")),
        "{e:?}"
    );
    assert!(
        e.lines().any(|l| l.contains("branch \\::warning::x")),
        "{e:?}"
    );
    assert_eq!(
        e.lines().filter(|l| l.starts_with("note:")).count(),
        2,
        "{e:?}"
    );
}

#[test]
fn notes_are_printed_alongside_violations() {
    let r = repo();
    r.append(
        "ledger.jsonl",
        &ledger_line("req_o1", "feat/other", Some("2026-01-02T00:00:00Z"), 5),
    );
    r.append("ledger.jsonl", "not json\n");
    let o = r.check();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("note: 1 new ledger line(s) recorded for branch feat/other"));
}

#[test]
fn prices_toml_difference_warns_in_each_direction_and_never_changes_the_exit_code() {
    const WARN: &str = "warning: .tally/prices.toml differs from base";
    // Added on head.
    let r = repo();
    r.write("prices.toml", "# synthetic\n");
    let o = r.check();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(err(&o).lines().any(|l| l == WARN), "{}", err(&o));

    // Changed, read from a committed head.
    let r = repo();
    r.write("prices.toml", "# one\n");
    commit_all(&r, "base prices");
    git(&r.root, &["branch", "-f", "main", "HEAD"]);
    r.write("prices.toml", "# two\n");
    commit_all(&r, "head prices");
    let o = r.run(&[
        "ledger",
        "check",
        "--base",
        "main",
        "--base-branch",
        "main",
        "--head",
        "HEAD",
        "--head-branch",
        BRANCH,
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(err(&o).lines().any(|l| l == WARN), "{}", err(&o));

    // Removed.
    git(&r.root, &["rm", "-q", ".tally/prices.toml"]);
    commit_all(&r, "drop prices");
    let o = r.check();
    assert!(err(&o).lines().any(|l| l == WARN), "{}", err(&o));

    // Identical: no warning.
    let r = repo();
    assert!(!err(&r.check()).contains("prices.toml"));
}

#[test]
fn allow_ledger_conflicts_without_from_ledger_is_a_usage_error() {
    let r = repo();
    let o = r.run(&["report", "--allow-ledger-conflicts"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains("--from-ledger"), "{}", err(&o));
}
