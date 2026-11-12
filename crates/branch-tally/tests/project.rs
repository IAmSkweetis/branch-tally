//! End-to-end tests for `project`: the ledger-wide report and the `--export-graph` SVG. Ledgers
//! are written by the real `record` into a tempdir, so no test touches a real repository's ledger.

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

fn recorded() -> (World, PathBuf) {
    let world = World::scenario("main-then-branch");
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let out = world.run(&["record", BRANCH, "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    (world, ledger)
}

#[test]
fn markdown_is_the_default_and_names_the_branch_with_a_mermaid_chart() {
    let (world, ledger) = recorded();
    let out = world.run(&["project", "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let md = stdout(&out);
    assert!(md.contains(BRANCH), "{md}");
    assert!(md.contains("mermaid"), "{md}");
    assert!(!md.contains('\r'));
}

#[test]
fn json_is_valid_and_lists_the_branch() {
    let (world, ledger) = recorded();
    let path = ledger.to_str().unwrap();
    for flag in ["--json", "--format=json"] {
        let out = world.run(&["project", "--ledger", path, flag]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert!(v.to_string().contains(BRANCH), "{v}");
    }
}

#[test]
fn md_flag_and_format_md_match_the_default() {
    let (world, ledger) = recorded();
    let path = ledger.to_str().unwrap();
    let default = stdout(&world.run(&["project", "--ledger", path]));
    for flag in ["--md", "--format=md"] {
        assert_eq!(
            stdout(&world.run(&["project", "--ledger", path, flag])),
            default
        );
    }
}

#[test]
fn json_and_md_conflict_is_a_usage_error() {
    let (world, ledger) = recorded();
    let out = world.run(&[
        "project",
        "--ledger",
        ledger.to_str().unwrap(),
        "--json",
        "--md",
    ]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn export_graph_writes_a_well_formed_svg_and_still_prints_the_report() {
    let (world, ledger) = recorded();
    let svg = world.home.join("scratch").join("graph.svg");
    let out = world.run(&[
        "project",
        "--ledger",
        ledger.to_str().unwrap(),
        "--export-graph",
        svg.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains(BRANCH));
    let text = fs::read_to_string(&svg).unwrap();
    assert!(text.trim_start().starts_with("<svg"), "{text}");
    assert!(text.trim_end().ends_with("</svg>"), "{text}");
    assert!(!text.contains('\r'));
}

#[test]
fn export_graph_works_with_json_output() {
    let (world, ledger) = recorded();
    let svg = world.home.join("scratch").join("graph.svg");
    let out = world.run(&[
        "project",
        "--json",
        "--ledger",
        ledger.to_str().unwrap(),
        "--export-graph",
        svg.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap();
    assert!(svg.exists());
}

#[test]
fn an_unwritable_export_path_fails_with_the_path_and_leaves_nothing() {
    let (world, ledger) = recorded();
    // The parent directory does not exist.
    let svg = world.home.join("no-such-dir").join("graph.svg");
    let out = world.run(&[
        "project",
        "--ledger",
        ledger.to_str().unwrap(),
        "--export-graph",
        svg.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains(&svg.display().to_string()), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    assert!(!svg.exists());
    assert!(!world.home.join("no-such-dir").exists());
}

#[test]
fn export_graph_needs_a_value() {
    let (world, ledger) = recorded();
    let out = world.run(&[
        "project",
        "--ledger",
        ledger.to_str().unwrap(),
        "--export-graph",
    ]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn a_missing_ledger_is_an_error_naming_the_path() {
    let world = World::scenario("main-then-branch");
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let out = world.run(&["project", "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains("no ledger at"), "{err}");
    assert!(err.contains(&ledger.display().to_string()), "{err}");
}

#[test]
fn the_default_ledger_is_the_checkouts() {
    let world = World::scenario("main-then-branch");
    assert_eq!(world.run(&["record", BRANCH]).status.code(), Some(0));
    let out = world.run(&["project"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains(BRANCH));
}

#[test]
fn repo_flag_selects_the_checkout_for_the_default_ledger() {
    let world = World::scenario("main-then-branch");
    assert_eq!(world.run(&["record", BRANCH]).status.code(), Some(0));
    let elsewhere = world.home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let out = world.run_in(
        &elsewhere,
        &["project", "--repo", world.repo.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains(BRANCH));
}

#[test]
fn config_flag_is_honoured_and_a_missing_config_is_an_error() {
    let (world, ledger) = recorded();
    let out = world.run(&[
        "project",
        "--ledger",
        ledger.to_str().unwrap(),
        "--config",
        world.home.join("absent.toml").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let cfg = world.write_config_file("c.toml", "format = \"json\"\n");
    let out = world.run(&[
        "project",
        "--ledger",
        ledger.to_str().unwrap(),
        "--config",
        cfg.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap();
}

#[test]
fn help_describes_the_flags() {
    let world = World::empty();
    let out = world.run(&["project", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = stdout(&out);
    for needle in [
        "--export-graph",
        "--ledger",
        "--repo",
        "--json",
        "--format",
        "--config",
    ] {
        assert!(help.contains(needle), "{needle}: {help}");
    }
    assert!(!help.contains("--mode"));
}
