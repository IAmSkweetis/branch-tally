//! End-to-end tests for `--timeline` and `--work-branch` (#7): the real binary, shared fixtures.

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, stderr, stdout};

#[test]
fn timeline_json_names_the_default_work_branch_rule() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["report", "feat/new", "--timeline", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let wb = v.get("workBranch").expect("timeline JSON has workBranch");
    assert_eq!(wb["kind"], "excludesDefault");
    assert!(wb["defaultBranch"].is_string());
    assert!(wb["source"].is_string());
    assert!(v["totalCalls"].as_u64().unwrap() > 0);
}

#[test]
fn timeline_markdown_says_timeline_mode() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["report", "feat/new", "--timeline"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("timeline mode"), "{}", stdout(&out));
}

#[test]
fn work_branch_pattern_is_shown_in_json_and_markdown() {
    let world = World::scenario("main-then-branch");
    let json = world.run(&[
        "report",
        "feat/new",
        "--json",
        "--timeline",
        "--work-branch",
        "^(feat|refactor)/",
    ]);
    assert_eq!(json.status.code(), Some(0), "{}", stderr(&json));
    let v: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(v["workBranch"]["kind"], "custom");
    assert_eq!(v["workBranch"]["pattern"], "^(feat|refactor)/");
    let md = world.run(&[
        "report",
        "feat/new",
        "--timeline",
        "--work-branch",
        "^(feat|refactor)/",
    ]);
    assert_eq!(md.status.code(), Some(0), "{}", stderr(&md));
    assert!(
        stdout(&md).contains("work branches: custom pattern `^(feat|refactor)/`"),
        "{}",
        stdout(&md)
    );
}

#[test]
fn work_branch_is_unanchored_as_written() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&[
        "report",
        "feat/new",
        "--timeline",
        "--work-branch",
        "new",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("\"pattern\": \"new\""));
}

#[test]
fn timeline_conflicts_with_strict() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["report", "feat/new", "--timeline", "--strict"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("--strict"), "{}", stderr(&out));
}

#[test]
fn work_branch_requires_timeline() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["report", "feat/new", "--work-branch", "^feat/"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("--timeline"), "{}", stderr(&out));
}

#[test]
fn invalid_work_branch_regex_is_an_argument_error() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["report", "feat/new", "--timeline", "--work-branch", "("]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains("--work-branch"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}

#[test]
fn claim_and_strict_json_carry_no_work_branch() {
    let world = World::scenario("main-then-branch");
    for extra in [None, Some("--strict")] {
        let mut args = vec!["report", "feat/new", "--json"];
        args.extend(extra);
        let out = world.run(&args);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(!stdout(&out).contains("workBranch"));
    }
}
