//! End-to-end tests for the capture-read notes on `report` and `record` (Governance II): bad
//! lines in `.tally/captures.jsonl` are counted and reported on stderr, never dropped silently.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, stderr, stdout};
use std::fs;

const BRANCH: &str = "feat/new";
const SECRET: &str = "hunter2-SECRET-line";

fn seed_captures(world: &World, body: &str) {
    let dir = world.repo.join(".tally");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("captures.jsonl"), body).unwrap();
}

fn bad_body() -> String {
    [
        format!("not json {SECRET}"),
        r#"{"v":2,"harness":"claude-code","kind":"stop"}"#.to_owned(),
        r#"{"v":1,"harness":"claude-code","kind":"teleport","session_id":"s","branch":"b","ts":"t"}"#
            .to_owned(),
    ]
    .join("\n")
        + "\n"
}

fn check(err: &str) {
    let note = err.lines().find(|l| l.contains("capture line"));
    assert!(note.is_some(), "no capture note: {err}");
    let note = note.unwrap();
    assert!(
        note.starts_with("note: skipped 3 capture line(s) in "),
        "{note}"
    );
    assert!(note.contains("1 malformed"), "{note}");
    assert!(note.contains("1 unsupported version"), "{note}");
    assert!(note.contains("1 unknown kind"), "{note}");
    assert!(note.contains("captures.jsonl"), "{note}");
    assert!(!err.contains(SECRET), "line content leaked: {err}");
}

#[test]
fn report_notes_bad_capture_lines() {
    let world = World::scenario("main-then-branch");
    seed_captures(&world, &bad_body());
    let out = world.run(&["report", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    check(&stderr(&out));
}

#[test]
fn record_notes_bad_capture_lines() {
    let world = World::scenario("main-then-branch");
    seed_captures(&world, &bad_body());
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let out = world.run(&["record", BRANCH, "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    check(&stderr(&out));
}

#[test]
fn from_ledger_with_live_notes_bad_capture_lines() {
    let world = World::scenario("main-then-branch");
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let path = ledger.to_str().unwrap();
    let rec = world.run(&["record", BRANCH, "--ledger", path]);
    assert_eq!(rec.status.code(), Some(0), "{}", stderr(&rec));
    seed_captures(&world, &bad_body());
    let out = world.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--with-live",
        "--ledger",
        path,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    check(&stderr(&out));
}

#[test]
fn clean_captures_leave_output_unchanged_and_bad_ones_only_touch_stderr() {
    let world = World::scenario("main-then-branch");
    let before = world.run(&["report", BRANCH]);
    seed_captures(&world, "");
    let after = world.run(&["report", BRANCH]);
    assert_eq!(stdout(&before), stdout(&after));
    assert_eq!(stderr(&before), stderr(&after));
    seed_captures(&world, &bad_body());
    let bad = world.run(&["report", BRANCH]);
    assert_eq!(stdout(&before), stdout(&bad));
}
