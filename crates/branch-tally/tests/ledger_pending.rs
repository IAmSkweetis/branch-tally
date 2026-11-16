//! End-to-end tests for pending ledger lines (decision 0006, #131): `record` on the default branch
//! writes them, `record` on another branch and `ledger resolve` resolve them by appending.

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
use serde_json::Value;
use std::fs;
use std::path::Path;

const BRANCH: &str = "feat/new";

fn ledger_path(world: &World) -> std::path::PathBuf {
    world.home.join("scratch").join("ledger.jsonl")
}

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// `record` on `main` (the repository's checkout), pending lines into the scratch ledger.
fn record_on_main(world: &World) -> std::process::Output {
    let ledger = ledger_path(world);
    world.run(&[
        "record",
        "--no-ledger-redact",
        "--ledger",
        ledger.to_str().unwrap(),
    ])
}

#[test]
fn record_on_the_default_branch_writes_pending_lines_and_says_so() {
    let world = World::scenario("main-then-branch");
    let out = record_on_main(&world);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let written = lines(&ledger_path(&world));
    assert!(!written.is_empty(), "{}", stderr(&out));
    assert!(written.iter().all(|l| l["pending"] == true));
    let n = written.len();
    let note = format!(
        "note: {n} pending line(s) written: usage is uncertain until a branch is cut and resolved"
    );
    assert!(stderr(&out).lines().any(|l| l == note), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!("appended {n}, skipped 0, branch main\n")
    );
}

#[test]
fn record_on_another_branch_prints_the_resolution_counts() {
    let world = World::scenario("main-then-branch");
    assert_eq!(record_on_main(&world).status.code(), Some(0));
    let ledger = ledger_path(&world);
    let out = world.run(&[
        "record",
        BRANCH,
        "--no-ledger-redact",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("note: resolved "), "{}", stderr(&out));
    assert!(!stderr(&out).contains("pending line(s) written"));
}

#[test]
fn ledger_resolve_dry_run_writes_nothing_and_says_would_resolve() {
    let world = World::scenario("main-then-branch");
    assert_eq!(record_on_main(&world).status.code(), Some(0));
    let ledger = ledger_path(&world);
    let before = fs::read(&ledger).unwrap();
    let out = world.run(&[
        "ledger",
        "resolve",
        "--branch",
        BRANCH,
        "--dry-run",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).starts_with("would resolve "),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains(&format!("branch {BRANCH}")));
    assert_eq!(fs::read(&ledger).unwrap(), before);
}

#[test]
fn ledger_resolve_appends_resolutions_and_a_second_run_appends_none() {
    let world = World::scenario("main-then-branch");
    assert_eq!(record_on_main(&world).status.code(), Some(0));
    let ledger = ledger_path(&world);
    let path = ledger.to_str().unwrap();
    let pending = lines(&ledger).len();
    let args = ["ledger", "resolve", "--branch", BRANCH, "--ledger", path];
    let out = world.run(&args);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("resolved "), "{}", stdout(&out));
    let after = lines(&ledger);
    assert!(after.len() > pending, "{}", stdout(&out));
    let resolution = after.last().unwrap();
    assert_eq!(resolution["branch"], BRANCH);
    assert_eq!(resolution["resolved"]["from"], "main");
    let again = world.run(&args);
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    assert_eq!(lines(&ledger).len(), after.len());
}

#[test]
fn ledger_resolve_on_the_default_branch_has_nothing_to_resolve() {
    let world = World::scenario("main-then-branch");
    let ledger = ledger_path(&world);
    let out = world.run(&["ledger", "resolve", "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "on the default branch: nothing to resolve\n");
    assert!(!ledger.exists());
}

#[test]
fn unplaceable_pending_lines_are_reported_as_uncertain_and_exit_0() {
    // Every call in this scenario was made on `main`; no work branch is ever cut.
    let world = World::scenario("unstamped-main");
    assert_eq!(record_on_main(&world).status.code(), Some(0));
    let ledger = ledger_path(&world);
    let before = lines(&ledger).len();
    let out = world.run(&[
        "ledger",
        "resolve",
        "--branch",
        "feat/x",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&format!(
            "note: {before} pending line(s) left unresolved, usage uncertain"
        )),
        "{}",
        stderr(&out)
    );
    assert_eq!(lines(&ledger).len(), before);
}

#[test]
fn ledger_resolve_help_lists_its_flags() {
    let world = World::empty();
    let out = world.run(&["ledger", "resolve", "--help"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let help = stdout(&out);
    for flag in ["--branch", "--dry-run", "--ledger", "--repo"] {
        assert!(help.contains(flag), "{flag} missing: {help}");
    }
}
