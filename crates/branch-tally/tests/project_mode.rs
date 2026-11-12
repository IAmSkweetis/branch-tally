//! `project` resolves the mode through the same config path as `report`.

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, stderr, stdout};
use std::fs;

const BRANCH: &str = "feat/new";

#[test]
fn project_row_uses_the_config_mode_like_report_from_ledger() {
    let world = World::scenario("main-then-branch");
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let ledger = ledger.to_str().unwrap();
    let out = world.run(&["record", BRANCH, "--ledger", ledger]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let config = world.home.join("scratch").join("strict.toml");
    fs::write(&config, "mode = \"strict\"\n").unwrap();
    let config = config.to_str().unwrap();

    // `report --json` carries no heuristic breakdown, so read it from the Markdown notes.
    let report = world.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--md",
        "--ledger",
        ledger,
        "--config",
        config,
    ]);
    assert_eq!(report.status.code(), Some(0), "{}", stderr(&report));
    let report = stdout(&report);
    let claimed = report
        .split("session claim ")
        .nth(1)
        .and_then(|rest| rest.split(" call(s)").next())
        .map_or(0, |n| n.parse::<u64>().unwrap());

    let project = world.run(&["project", "--json", "--ledger", ledger, "--config", config]);
    assert_eq!(project.status.code(), Some(0), "{}", stderr(&project));
    let project: serde_json::Value = serde_json::from_str(&stdout(&project)).unwrap();
    let row = &project["branches"][0];
    assert_eq!(row["branch"], BRANCH);
    assert_eq!(
        row["heuristics"]["sessionClaim"]["calls"].as_u64(),
        Some(claimed),
        "report said: {report}"
    );
}
