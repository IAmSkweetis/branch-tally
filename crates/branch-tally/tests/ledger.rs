//! End-to-end tests for `record` and `report --from-ledger`, always with `--ledger` pointing into
//! a tempdir so no test touches a real repository's ledger.

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
use std::path::PathBuf;

const BRANCH: &str = "feat/new";

fn ledger_path(world: &World) -> PathBuf {
    world.home.join("scratch").join("ledger.jsonl")
}

/// Blanks the session origin, which the ledger does not store yet (reported to the orchestrator
/// as `needs_core_api`): a ledger-backed report shows `unknown` where a live one shows `main` or
/// a worktree name, and Markdown labels that session's `Where` cell `worktree`. Every number,
/// name and span still has to match exactly.
fn without_origin(text: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(mut v) => {
            for s in v["sessions"].as_array_mut().into_iter().flatten() {
                s["origin"] = "".into();
            }
            v.to_string()
        }
        Err(_) => text
            .replace("| main checkout |", "| <where> |")
            .replace("| worktree |", "| <where> |"),
    }
}

fn recorded() -> (World, PathBuf) {
    let world = World::scenario("main-then-branch");
    let ledger = ledger_path(&world);
    let out = world.run(&["record", BRANCH, "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    (world, ledger)
}

#[test]
fn record_appends_then_a_second_run_appends_nothing() {
    let world = World::scenario("main-then-branch");
    let ledger = ledger_path(&world);
    let path = ledger.to_str().unwrap();
    let first = world.run(&["record", BRANCH, "--ledger", path]);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(
        stdout(&first),
        "appended 5, skipped 0, branch feat/new\n",
        "{}",
        stderr(&first)
    );
    let lines = fs::read_to_string(&ledger).unwrap().lines().count();
    assert_eq!(lines, 5);
    let second = world.run(&["record", BRANCH, "--ledger", path]);
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(stdout(&second), "appended 0, skipped 5, branch feat/new\n");
    assert_eq!(fs::read_to_string(&ledger).unwrap().lines().count(), 5);
}

#[test]
fn record_writes_no_message_content_and_no_cwd() {
    let (_world, ledger) = recorded();
    let text = fs::read_to_string(ledger).unwrap();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(v.get("cwd").is_none());
        assert!(v.get("message").is_none());
        assert!(v.get("content").is_none());
    }
}

#[test]
fn record_defaults_to_the_checkout_ledger() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["record", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let default = world.repo.join(".tally").join("ledger.jsonl");
    assert_eq!(fs::read_to_string(default).unwrap().lines().count(), 5);
}

#[test]
fn record_with_nothing_selected_appends_nothing_and_says_so() {
    let world = World::scenario("main-then-branch");
    let ledger = ledger_path(&world);
    let out = world.run(&[
        "record",
        "no/such-branch",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "appended 0, skipped 0, branch no/such-branch\n"
    );
}

#[test]
fn from_ledger_equals_the_live_report_in_json_and_markdown() {
    let (world, ledger) = recorded();
    let path = ledger.to_str().unwrap();
    for fmt in ["--json", "--md"] {
        let live = world.run(&["report", BRANCH, fmt]);
        let from = world.run(&["report", BRANCH, fmt, "--from-ledger", "--ledger", path]);
        assert_eq!(live.status.code(), Some(0), "{}", stderr(&live));
        assert_eq!(from.status.code(), Some(0), "{}", stderr(&from));
        assert_eq!(
            without_origin(&stdout(&live)),
            without_origin(&stdout(&from)),
            "{fmt}"
        );
    }
}

#[test]
fn from_ledger_works_with_no_transcripts_at_all() {
    let (world, ledger) = recorded();
    fs::remove_dir_all(world.projects()).unwrap();
    let out = world.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("feat/new"));
}

#[test]
fn from_ledger_notes_that_unattributed_counts_are_unavailable() {
    let (world, ledger) = recorded();
    let out = world.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert!(
        stderr(&out)
            .lines()
            .any(|l| l == "note: unattributed counts unavailable from the ledger"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn with_live_does_not_double_count_and_drops_the_unavailable_note() {
    let (world, ledger) = recorded();
    let path = ledger.to_str().unwrap();
    let live = world.run(&["report", BRANCH, "--json"]);
    let both = world.run(&[
        "report",
        BRANCH,
        "--json",
        "--from-ledger",
        "--with-live",
        "--ledger",
        path,
    ]);
    assert_eq!(both.status.code(), Some(0), "{}", stderr(&both));
    assert_eq!(
        without_origin(&stdout(&live)),
        without_origin(&stdout(&both))
    );
    assert!(!stderr(&both).contains("unattributed counts unavailable"));
}

#[test]
fn with_live_alone_is_an_argument_error() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["report", BRANCH, "--with-live"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
}

#[test]
fn a_missing_ledger_is_an_error_naming_the_path() {
    let world = World::scenario("main-then-branch");
    let ledger = ledger_path(&world);
    let out = world.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains("no ledger at"), "{err}");
    assert!(err.contains(&ledger.display().to_string()), "{err}");
}

#[test]
fn mixed_modes_and_ledger_counts_are_noted() {
    let (world, ledger) = recorded();
    let path = ledger.to_str().unwrap();
    // Same branch, other mode: new keys are only added if the ledger keys on mode-independent
    // ids, so add a hand-made second-mode line and a malformed line instead.
    let first = fs::read_to_string(&ledger).unwrap();
    let line = first.lines().next().unwrap();
    let mut v: serde_json::Value = serde_json::from_str(line).unwrap();
    v["request_id"] = "req-mixed-mode".into();
    v["mode"] = "strict".into();
    let mut text = first.clone();
    text.push_str(&v.to_string());
    text.push_str("\nnot json\n");
    fs::write(&ledger, text).unwrap();
    let out = world.run(&["report", BRANCH, "--from-ledger", "--ledger", path]);
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(0), "{err}");
    assert!(
        err.lines()
            .any(|l| l.starts_with("note: ledger mixes modes")),
        "{err}"
    );
    assert!(
        err.lines()
            .any(|l| l.starts_with("note: skipped") && l.contains("1 malformed")),
        "{err}"
    );
}

#[test]
fn from_ledger_honours_pr_dry_run_flags_cost_and_mode() {
    let (world, ledger) = recorded();
    let path = ledger.to_str().unwrap();
    let out = world.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--ledger",
        path,
        "--no-cost",
        "--mode",
        "claim",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let live = world.run(&["report", BRANCH, "--no-cost"]);
    assert_eq!(
        without_origin(&stdout(&out)),
        without_origin(&stdout(&live))
    );
}
