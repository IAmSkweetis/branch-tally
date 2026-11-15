//! End-to-end tests for ledger redaction (#67, decision 0005): `record`'s keyed-hash default, its
//! flags and config keys, `ledger redact`, and the mixed-ledger note. Synthetic fixtures only;
//! every ledger lives in a tempdir.

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
use std::path::{Path, PathBuf};

const BRANCH: &str = "feat/x";
const SCENARIO: &str = "subagent";

/// A world whose repository has opted in (`.tally/` exists).
fn opted_in() -> World {
    let world = World::scenario(SCENARIO);
    fs::create_dir(world.repo.join(".tally")).unwrap();
    world
}

fn ledger_of(world: &World) -> PathBuf {
    world.repo.join(".tally").join("ledger.jsonl")
}

fn salt_of(world: &World) -> PathBuf {
    world.repo.join(".tally").join("salt")
}

fn record(world: &World, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["record", BRANCH];
    args.extend_from_slice(extra);
    let out = world.run(&args);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    out
}

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// No raw id of the scenario survives anywhere in the file's text.
fn assert_no_raw_ids(path: &Path) {
    let text = fs::read_to_string(path).unwrap();
    for raw in ["req_1", "req_a1", "req_a2", "\"s1\"", "\"a1\"", "agent-a1"] {
        assert!(!text.contains(raw), "raw id {raw} survived in the ledger");
    }
}

/// The report's totals for the branch, from the ledger at `ledger`.
fn totals(world: &World, ledger: &Path) -> Value {
    let out = world.run(&[
        "report",
        BRANCH,
        "--json",
        "--from-ledger",
        "--ledger",
        ledger.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(
        v["totalCalls"].as_u64().unwrap() > 0,
        "no calls in the report"
    );
    serde_json::json!({
        "calls": v["totalCalls"],
        "cost": v["totalCostUSD"],
        "models": v["models"],
    })
}

#[test]
fn default_record_hashes_ids_marks_lines_and_creates_the_salt() {
    let world = opted_in();
    record(&world, &[]);
    assert!(salt_of(&world).is_file(), "the salt was not created");
    let entries = lines(&ledger_of(&world));
    assert_eq!(entries.len(), 3);
    for e in &entries {
        assert!(
            e["redaction"].as_str().is_some_and(|f| f.len() == 16),
            "{e}"
        );
        let request = e["request_id"].as_str().unwrap();
        assert_eq!(request.len(), 32, "{e}");
        assert!(request.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(e["session_id"].as_str().unwrap().len(), 32, "{e}");
    }
    assert!(
        entries
            .iter()
            .any(|e| e["agent_id"].as_str().is_some_and(|a| a.len() == 32)),
        "no hashed agent id"
    );
    assert_no_raw_ids(&ledger_of(&world));
}

#[test]
fn no_ledger_redact_writes_raw_ids_and_no_salt() {
    let world = opted_in();
    record(&world, &["--no-ledger-redact"]);
    assert!(!salt_of(&world).exists());
    let entries = lines(&ledger_of(&world));
    assert!(entries.iter().all(|e| e.get("redaction").is_none()));
    assert!(entries.iter().any(|e| e["request_id"] == "req_1"));
}

#[test]
fn the_config_file_can_turn_ledger_redaction_off_and_a_flag_beats_it() {
    let world = opted_in();
    let cfg = world.write_config_file("cfg.toml", "ledger-redact = false\n");
    record(&world, &["--config", cfg.to_str().unwrap()]);
    assert!(!salt_of(&world).exists());
    assert!(lines(&ledger_of(&world))[0].get("redaction").is_none());

    let world = opted_in();
    let cfg = world.write_config_file("cfg.toml", "ledger-redact = false\n");
    record(
        &world,
        &["--config", cfg.to_str().unwrap(), "--ledger-redact"],
    );
    assert!(salt_of(&world).is_file());
}

#[test]
fn no_tally_dir_writes_nothing_and_creates_no_salt() {
    let world = World::scenario(SCENARIO);
    let out = record(&world, &[]);
    assert!(
        stdout(&out).starts_with("no .tally/ directory"),
        "{}",
        stdout(&out)
    );
    assert!(!world.repo.join(".tally").exists());
}

#[test]
fn redaction_with_an_explicit_ledger_and_no_tally_dir_is_a_fixed_error() {
    let world = World::scenario(SCENARIO);
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let out = world.run(&["record", BRANCH, "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("no-ledger-redact"),
        "{}",
        stderr(&out)
    );
    assert!(!ledger.exists());
}

#[test]
fn names_are_hashed_only_under_the_names_opt_in() {
    for (flags, hashed) in [(vec![], false), (vec!["--ledger-redact-names"], true)] {
        let world = opted_in();
        record(&world, &["--no-ledger-redact"]);
        let path = ledger_of(&world);
        let text = fs::read_to_string(&path).unwrap().replacen(
            "{\"v\"",
            "{\"agent\":\"reviewer-x\",\"skill\":\"skill-x\",\"v\"",
            1,
        );
        fs::write(&path, text).unwrap();
        let mut args = vec!["ledger", "redact"];
        args.extend(flags);
        let out = world.run(&args);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(!body.contains("reviewer-x"), hashed, "agent name");
        assert_eq!(!body.contains("skill-x"), hashed, "skill name");
    }
}

#[test]
fn two_clones_sharing_the_committed_salt_total_the_same_as_an_unredacted_run() {
    let a = opted_in();
    record(&a, &[]);
    let b = opted_in();
    fs::copy(salt_of(&a), salt_of(&b)).unwrap();
    record(&b, &[]);
    let union = a.home.join("union.jsonl");
    let mut text = fs::read_to_string(ledger_of(&a)).unwrap();
    text.push_str(&fs::read_to_string(ledger_of(&b)).unwrap());
    fs::write(&union, text).unwrap();

    let plain = opted_in();
    record(&plain, &["--no-ledger-redact"]);
    let expected = totals(&plain, &ledger_of(&plain));
    assert_eq!(totals(&a, &union), expected);

    let out = a.run(&[
        "report",
        BRANCH,
        "--from-ledger",
        "--ledger",
        union.to_str().unwrap(),
    ]);
    assert!(!stderr(&out).contains("unredacted"), "{}", stderr(&out));
}

#[test]
fn ledger_redact_then_record_keeps_totals_and_notes_nothing() {
    let world = opted_in();
    record(&world, &["--no-ledger-redact"]);
    let before = totals(&world, &ledger_of(&world));

    let out = world.run(&["ledger", "redact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "redacted 3 line(s), 0 already redacted, 0 unparseable left as-is\n"
    );
    assert!(salt_of(&world).is_file());
    assert_no_raw_ids(&ledger_of(&world));

    let again = record(&world, &[]);
    assert_eq!(stdout(&again), "appended 0, skipped 3, branch feat/x\n");
    assert_eq!(lines(&ledger_of(&world)).len(), 3);
    assert_eq!(totals(&world, &ledger_of(&world)), before);

    let report = world.run(&["report", BRANCH, "--from-ledger"]);
    assert_eq!(report.status.code(), Some(0), "{}", stderr(&report));
    assert!(
        !stderr(&report).contains("unredacted"),
        "{}",
        stderr(&report)
    );
}

#[test]
fn ledger_redact_is_idempotent_and_honours_an_explicit_ledger() {
    let world = opted_in();
    record(&world, &["--no-ledger-redact"]);
    let path = ledger_of(&world);
    let out = world.run(&["ledger", "redact", "--ledger", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_no_raw_ids(&path);
    let second = world.run(&["ledger", "redact"]);
    assert_eq!(
        stdout(&second),
        "redacted 0 line(s), 3 already redacted, 0 unparseable left as-is\n"
    );
}

#[test]
fn ledger_redact_runs_with_ledger_redact_off_and_leaves_unparseable_lines() {
    let world = opted_in();
    record(&world, &["--no-ledger-redact"]);
    let mut text = fs::read_to_string(ledger_of(&world)).unwrap();
    text.push_str("not json\n");
    fs::write(ledger_of(&world), text).unwrap();
    let cfg = world.write_config_file("cfg.toml", "ledger-redact = false\n");
    let out = world.run(&["ledger", "redact", "--config", cfg.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "redacted 3 line(s), 0 already redacted, 1 unparseable left as-is\n"
    );
    assert!(
        fs::read_to_string(ledger_of(&world))
            .unwrap()
            .ends_with("not json\n")
    );
}

#[test]
fn ledger_redact_with_no_tally_dir_says_so_and_creates_nothing() {
    let world = World::scenario(SCENARIO);
    let out = world.run(&["ledger", "redact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("no .tally/ directory"),
        "{}",
        stdout(&out)
    );
    assert!(!world.repo.join(".tally").exists());
}

#[test]
fn a_mixed_ledger_prints_the_note_for_report_and_project() {
    let world = opted_in();
    record(&world, &["--no-ledger-redact"]);
    record(&world, &[]);
    for args in [vec!["report", BRANCH, "--from-ledger"], vec!["project"]] {
        let out = world.run(&args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        let err = stderr(&out);
        assert!(err.contains("3 unredacted line(s)"), "{err}");
        assert!(err.contains("3 redacted line(s)"), "{err}");
        assert!(err.contains("1 salt(s)"), "{err}");
        assert!(err.contains("counted twice"), "{err}");
        assert!(err.contains("branch-tally ledger redact"), "{err}");
    }
}
