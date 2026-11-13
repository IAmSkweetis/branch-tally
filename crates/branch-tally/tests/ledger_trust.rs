//! End-to-end tests for the F-03 trust gates: conflicting ledger lines stop a ledger-reading run
//! with exit 3 unless `--allow-ledger-conflicts`, and a repository price override is announced on
//! stderr. Ledgers and overrides are synthetic, in temp directories.

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
use std::path::{Path, PathBuf};

const BRANCH: &str = "feat/new";
const FLAG: &str = "--allow-ledger-conflicts";

fn recorded() -> (World, PathBuf) {
    let world = World::scenario("main-then-branch");
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let out = world.run(&["record", BRANCH, "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    (world, ledger)
}

/// A recorded ledger whose first line is then re-added with different usage and a later
/// `recorded_at`: one conflicting request.
fn conflicted() -> (World, PathBuf) {
    let (world, ledger) = recorded();
    let text = fs::read_to_string(&ledger).unwrap();
    let mut line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    line["usage"]["output"] = serde_json::json!(line["usage"]["output"].as_u64().unwrap() + 1000);
    line["recorded_at"] = serde_json::json!("2999-01-01T00:00:00Z");
    fs::write(&ledger, format!("{text}{line}\n")).unwrap();
    (world, ledger)
}

fn readers(ledger: &Path) -> Vec<Vec<String>> {
    let l = ledger.to_str().unwrap();
    [
        vec!["report", BRANCH, "--from-ledger", "--ledger", l],
        vec!["report", BRANCH, "--from-ledger", "--json", "--ledger", l],
        vec!["project", "--ledger", l],
        vec!["project", "--json", "--ledger", l],
    ]
    .into_iter()
    .map(|v| v.into_iter().map(str::to_owned).collect())
    .collect()
}

fn run(world: &World, args: &[String], extra: &[&str]) -> std::process::Output {
    let mut all: Vec<&str> = args.iter().map(String::as_str).collect();
    all.extend_from_slice(extra);
    world.run(&all)
}

#[test]
fn conflicting_ledger_lines_stop_every_reader_with_exit_3() {
    let (world, ledger) = conflicted();
    for args in readers(&ledger) {
        let out = run(&world, &args, &[]);
        assert_eq!(out.status.code(), Some(3), "{args:?}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(err.contains(FLAG), "{err}");
        assert!(err.contains('1'), "{err}");
        assert!(stdout(&out).is_empty(), "{args:?}: nothing on stdout");
    }
}

#[test]
fn the_flag_lets_the_run_proceed_and_still_reports_the_count() {
    let (world, ledger) = conflicted();
    for args in readers(&ledger) {
        let out = run(&world, &args, &[FLAG]);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("conflicting"), "{}", stderr(&out));
        if args.iter().any(|a| a == "--json") {
            serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap();
        }
    }
}

#[test]
fn the_conflict_message_has_counts_only() {
    let (world, ledger) = conflicted();
    let text = fs::read_to_string(&ledger).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let id = first["request_id"].as_str().unwrap().to_owned();
    for args in readers(&ledger) {
        for extra in [&[][..], &[FLAG][..]] {
            let err = stderr(&run(&world, &args, extra));
            assert!(!err.contains(&id), "{err}");
            assert!(!err.contains("request_id"), "{err}");
        }
    }
}

#[test]
fn a_clean_ledger_needs_no_flag() {
    let (world, ledger) = recorded();
    let out = world.run(&["project", "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

const OVERRIDE: &str =
    "as_of = \"2030-01-02\"\n\n[models.\"claude-opus-5\"]\ninput = 1.0\noutput = 1.0\n";

fn write_override(world: &World) {
    let dir = world.repo.join(".tally");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("prices.toml"), OVERRIDE).unwrap();
}

#[test]
fn a_price_override_is_announced_on_stderr_in_every_output_mode() {
    let world = World::scenario("priced-models");
    let quiet = world.run(&["report", "main"]);
    assert!(
        !stderr(&quiet).contains("price override"),
        "{}",
        stderr(&quiet)
    );

    write_override(&world);
    for args in [&["report", "main"][..], &["report", "main", "--json"][..]] {
        let out = world.run(args);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("repository price override"),
            "{args:?}: {}",
            stderr(&out)
        );
        if args.contains(&"--json") {
            serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap();
        }
        assert!(!stdout(&out).contains("price override"));
    }
}

#[test]
fn a_price_override_is_announced_by_project_too() {
    let (world, ledger) = recorded();
    write_override(&world);
    let out = world.run(&["project", "--json", "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("repository price override"));
    serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap();
}
