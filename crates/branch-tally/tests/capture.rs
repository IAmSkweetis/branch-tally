//! End-to-end tests for `capture`, the hook entry point. Each runs the real binary with a hook
//! payload on stdin against a throwaway git repository.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup fails loudly"
)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_branch-tally");
const SECRET: &str = "hunter2-SECRET-prompt";

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

fn repo() -> Repo {
    let scratch = tempfile::Builder::new()
        .prefix("bt-capture-")
        .tempdir()
        .unwrap();
    let root = branch_tally_core::git::normalize_path(scratch.path()).join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["commit", "-q", "--allow-empty", "-m", "init"]);
    Repo {
        _scratch: scratch,
        root,
    }
}

/// A repository that has opted in to `.tally/`: capture never creates the directory (#94).
fn opted_in() -> Repo {
    let r = repo();
    fs::create_dir(r.root.join(".tally")).unwrap();
    r
}

impl Repo {
    fn add_worktree(&self, dir_name: &str, branch: &str) -> PathBuf {
        let path = self.root.join(".claude").join("worktrees").join(dir_name);
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                path.to_str().unwrap(),
            ],
        );
        path
    }

    fn captures(&self) -> Vec<Value> {
        let path = self.root.join(".tally").join("captures.jsonl");
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn run(payload: &str, cwd: &Path) -> Output {
    let mut child = Command::new(BIN)
        .arg("capture")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn payload(event: &str, cwd: Option<&Path>, agent: Option<&str>) -> String {
    let mut v = json!({
        "hook_event_name": event,
        "session_id": "sess-1",
        "prompt": SECRET,
    });
    if let Some(cwd) = cwd {
        v["cwd"] = json!(cwd.to_str().unwrap());
    }
    if let Some(agent) = agent {
        v["agent_id"] = json!(agent);
    }
    v.to_string()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn session_start_writes_one_id_only_event_quietly() {
    let r = opted_in();
    let out = run(&payload("SessionStart", Some(&r.root), None), &r.root);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "");
    let events = r.captures();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e["kind"], "session-start");
    assert_eq!(e["session_id"], "sess-1");
    assert_eq!(e["branch"], "main");
    assert!(e.get("worktree_branch").is_none());
    assert!(e.get("agent_id").is_none());
    let ts = e["ts"].as_str().unwrap();
    assert!(
        ts.ends_with('Z') || ts.contains('+'),
        "ts has an offset: {ts}"
    );
    let raw = fs::read_to_string(r.root.join(".tally/captures.jsonl")).unwrap();
    assert!(!raw.contains(SECRET));
    assert!(!raw.contains("bt-capture-"), "no path stored");
}

#[test]
fn stop_maps_to_stop_kind() {
    let r = opted_in();
    let out = run(&payload("Stop", Some(&r.root), None), &r.root);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(r.captures()[0]["kind"], "stop");
}

#[test]
fn subagent_stop_names_the_agent_worktree_branch() {
    let r = opted_in();
    r.add_worktree("agent-abc123", "feat/from-agent");
    let out = run(
        &payload("SubagentStop", Some(&r.root), Some("abc123")),
        &r.root,
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let e = &r.captures()[0];
    assert_eq!(e["kind"], "subagent-stop");
    assert_eq!(e["agent_id"], "abc123");
    assert_eq!(e["branch"], "main");
    assert_eq!(e["worktree_branch"], "feat/from-agent");
}

#[test]
fn agent_worktree_name_matches_case_insensitively() {
    let r = opted_in();
    r.add_worktree("agent-AbC123", "feat/case");
    let out = run(
        &payload("SubagentStop", Some(&r.root), Some("abc123")),
        &r.root,
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(r.captures()[0]["worktree_branch"], "feat/case");
}

#[test]
fn worktree_session_captures_land_in_the_main_checkout() {
    let r = opted_in();
    let wt = r.add_worktree("agent-zzz", "feat/wt");
    let out = run(&payload("Stop", Some(&wt), None), &wt);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(!wt.join(".tally").exists());
    let events = r.captures();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["branch"], "feat/wt");
}

#[test]
fn subagent_stop_from_a_worktree_cwd_uses_that_worktrees_branch() {
    let r = opted_in();
    let wt = r.add_worktree("not-an-agent-dir", "feat/other");
    let out = run(&payload("SubagentStop", Some(&wt), Some("nomatch")), &wt);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let e = &r.captures()[0];
    assert_eq!(e["branch"], "feat/other");
    assert_eq!(e["worktree_branch"], "feat/other");
}

#[test]
fn removed_worktree_gives_no_worktree_branch() {
    let r = opted_in();
    let wt = r.add_worktree("agent-gone", "feat/gone");
    git(
        &r.root,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    let out = run(
        &payload("SubagentStop", Some(&r.root), Some("gone")),
        &r.root,
    );
    assert_eq!(out.status.code(), Some(0));
    let e = &r.captures()[0];
    assert!(e.get("worktree_branch").is_none(), "never guess: {e}");
}

#[test]
fn main_checkout_cwd_with_unknown_agent_has_no_worktree_branch() {
    let r = opted_in();
    let out = run(
        &payload("SubagentStop", Some(&r.root), Some("nobody")),
        &r.root,
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(r.captures()[0].get("worktree_branch").is_none());
}

#[test]
fn absent_cwd_falls_back_to_the_process_cwd() {
    let r = opted_in();
    let out = run(&payload("SessionStart", None, None), &r.root);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(r.captures()[0]["branch"], "main");
}

#[test]
fn session_end_and_subagent_start_are_ignored() {
    let r = repo();
    for (event, agent) in [("SessionEnd", None), ("SubagentStart", Some("a1"))] {
        let out = run(&payload(event, Some(&r.root), agent), &r.root);
        assert_eq!(out.status.code(), Some(0), "{event}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
    }
    assert!(!r.root.join(".tally").exists());
}

#[test]
fn not_a_repo_fails_quietly_without_echoing_the_cwd() {
    let scratch = tempfile::Builder::new()
        .prefix("bt-norepo-")
        .tempdir()
        .unwrap();
    let dir = branch_tally_core::git::normalize_path(scratch.path());
    let out = run(&payload("Stop", Some(&dir), None), &dir);
    let code = out.status.code();
    assert!(code.is_some_and(|c| c != 0 && c != 2), "{code:?}");
    let err = text(&out.stderr);
    assert_ne!(err, "");
    assert!(!err.contains("bt-norepo-"), "{err}");
    assert!(!err.contains(SECRET), "{err}");
    assert!(!dir.join(".tally").exists());
}

#[test]
fn malformed_payload_fails_without_echoing_it() {
    let r = repo();
    let bad = format!(r#"{{"prompt": "{SECRET}", "cwd": "{}""#, "nope");
    let out = run(&bad, &r.root);
    let code = out.status.code();
    assert!(code.is_some_and(|c| c != 0 && c != 2), "{code:?}");
    let err = text(&out.stderr);
    assert_ne!(err, "");
    assert!(!err.contains(SECRET) && !err.contains("nope"), "{err}");
    assert!(!r.root.join(".tally").exists());
}

#[test]
fn detached_head_records_nothing() {
    let r = repo();
    git(&r.root, &["checkout", "-q", "--detach"]);
    let out = run(&payload("Stop", Some(&r.root), None), &r.root);
    let code = out.status.code();
    assert!(code.is_some_and(|c| c != 0 && c != 2), "{code:?}");
    assert!(!r.root.join(".tally").exists());
}

#[test]
fn help_says_it_is_for_the_plugin_hooks() {
    let out = Command::new(BIN)
        .args(["capture", "--help"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let help = text(&out.stdout);
    assert!(help.contains("hook"), "{help}");
    assert!(help.contains("plugin"), "{help}");
}

/// The `/x/...` Git Bash form of a drive path only means something on Windows, so this is gated.
/// The process cwd is a different directory, so the fallback cannot mask a failed translation.
#[cfg(windows)]
#[test]
fn git_bash_drive_form_cwd_is_captured() {
    let r = repo();
    let bash = branch_tally_core::paths::git_bash_form(&r.root.to_string_lossy()).unwrap();
    assert!(bash.starts_with('/'), "{bash}");
    let elsewhere = tempfile::Builder::new()
        .prefix("bt-elsewhere-")
        .tempdir()
        .unwrap();
    let payload = json!({
        "hook_event_name": "SessionStart",
        "session_id": "sess-1",
        "cwd": bash,
    })
    .to_string();
    let out = run(&payload, elsewhere.path());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let events = r.captures();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["branch"], "main");
    assert!(!text(&out.stderr).contains("bt-capture-"));
}
