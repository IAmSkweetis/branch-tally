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
        "redacted 3 line(s), 0 restamped, 0 already redacted, 0 unparseable left as-is\n"
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
        "redacted 0 line(s), 0 restamped, 3 already redacted, 0 unparseable left as-is\n"
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
        "redacted 3 line(s), 0 restamped, 0 already redacted, 1 unparseable left as-is\n"
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

fn live_report(world: &World) -> std::process::Output {
    let out = world.run(&["report", BRANCH, "--from-ledger", "--with-live"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    out
}

const NO_MATCH: &str = "could not be matched against the redacted ledger lines";

#[test]
fn live_matching_failures_are_noted_and_a_match_is_silent() {
    let world = opted_in();
    record(&world, &[]);
    let quiet = live_report(&world);
    assert!(!stderr(&quiet).contains(NO_MATCH), "{}", stderr(&quiet));

    let salt = salt_of(&world);
    let good = fs::read_to_string(&salt).unwrap();
    let ledger = ledger_of(&world);
    let original = fs::read_to_string(&ledger).unwrap();

    let cases: [(&str, &str); 4] = [
        ("none", "no salt in .tally/"),
        ("garbage", "the salt is unreadable"),
        ("other", "the local salt matches none of the redacted lines"),
        (
            "partial",
            "the local salt matches only some of the redacted lines",
        ),
    ];
    for (case, expected) in cases {
        fs::write(&ledger, &original).unwrap();
        match case {
            "none" => fs::remove_file(&salt).unwrap(),
            "garbage" => fs::write(&salt, "not a salt\n").unwrap(),
            "other" => fs::write(&salt, "ab".repeat(32) + "\n").unwrap(),
            _ => {
                let first = lines(&ledger)[0]["redaction"].as_str().unwrap().to_owned();
                fs::write(&ledger, original.replacen(&first, &"0".repeat(16), 1)).unwrap();
            }
        }
        let out = live_report(&world);
        let err = stderr(&out);
        assert!(err.contains(NO_MATCH), "{case}: {err}");
        assert!(err.contains(expected), "{case}: {err}");
        assert!(err.contains("count a call twice"), "{case}: {err}");
        assert!(!err.contains(&"ab".repeat(8)), "{case}: leaked salt text");
        // Without --with-live nothing is unioned, so nothing is noted.
        let ledger_only = world.run(&["report", BRANCH, "--from-ledger"]);
        assert!(!stderr(&ledger_only).contains(NO_MATCH), "{case}");
        fs::write(&salt, &good).unwrap();
    }
}

/// A real linked worktree of the world's repository, with its own `.tally/` when asked.
fn linked_worktree(world: &World, with_tally: bool) -> PathBuf {
    let wt = world.home.join("linked-wt");
    let out = std::process::Command::new("git")
        .args(["worktree", "add", "-q", "-b", "chore/wt"])
        .arg(&wt)
        .current_dir(&world.repo)
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git worktree add failed");
    let wt = common::canon(&wt);
    if with_tally {
        fs::create_dir(wt.join(".tally")).unwrap();
    }
    wt
}

#[test]
fn a_worktree_with_its_own_tally_dir_hashes_and_keeps_its_salt_there() {
    let world = World::scenario(SCENARIO);
    let wt = linked_worktree(&world, true);
    let out = world.run_in(&wt, &["record", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let ledger = wt.join(".tally").join("ledger.jsonl");
    assert!(wt.join(".tally").join("salt").is_file());
    assert!(!world.repo.join(".tally").exists(), "main was touched");
    for e in lines(&ledger) {
        assert!(e["redaction"].is_string(), "raw line written: {e}");
    }
    assert_no_raw_ids(&ledger);
}

#[test]
fn a_worktree_without_a_salt_copies_the_main_checkouts_salt() {
    let world = opted_in();
    record(&world, &[]);
    let wt = linked_worktree(&world, true);
    let out = world.run_in(&wt, &["record", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        fs::read_to_string(wt.join(".tally").join("salt")).unwrap(),
        fs::read_to_string(salt_of(&world)).unwrap()
    );
    let main_lines = lines(&ledger_of(&world));
    let wt_lines = lines(&wt.join(".tally").join("ledger.jsonl"));
    assert_eq!(main_lines[0]["request_id"], wt_lines[0]["request_id"]);
}

#[test]
fn nothing_written_means_no_salt_anywhere() {
    // Main has `.tally/` but the worktree does not: the default ledger is the worktree's, so
    // nothing is written and main must not gain a salt.
    let world = opted_in();
    let wt = linked_worktree(&world, false);
    let out = world.run_in(&wt, &["record", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("no .tally/ directory"));
    assert!(!salt_of(&world).exists());
    assert!(!wt.join(".tally").exists());
    let redact = world.run_in(&wt, &["ledger", "redact"]);
    assert_eq!(redact.status.code(), Some(0), "{}", stderr(&redact));
    assert!(!salt_of(&world).exists());
}

#[test]
fn ledger_redact_in_a_worktree_uses_the_worktrees_salt() {
    let world = World::scenario(SCENARIO);
    let wt = linked_worktree(&world, true);
    let out = world.run_in(&wt, &["record", BRANCH, "--no-ledger-redact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = world.run_in(&wt, &["ledger", "redact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(wt.join(".tally").join("salt").is_file());
    assert!(!world.repo.join(".tally").exists());
    assert_no_raw_ids(&wt.join(".tally").join("ledger.jsonl"));
}

#[test]
fn ledger_redact_with_an_explicit_ledger_and_no_tally_dir_has_its_own_text() {
    let world = World::scenario(SCENARIO);
    let ledger = world.home.join("scratch").join("ledger.jsonl");
    let out = world.run(&["ledger", "redact", "--ledger", ledger.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("ledger redact needs a .tally/"), "{err}");
    assert!(!err.contains("--no-ledger-redact"), "{err}");
}

#[test]
fn ledger_redact_help_names_worktree_names() {
    let world = World::scenario(SCENARIO);
    let out = world.run(&["ledger", "redact", "--help"]);
    let help = stdout(&out);
    assert!(help.contains("worktree-agent-"), "{help}");
    assert!(help.contains("worktree names"), "{help}");
}

#[test]
fn ledger_redact_warns_about_unparseable_lines_and_names_backfill() {
    let world = opted_in();
    record(&world, &[]);
    let mut text = fs::read_to_string(ledger_of(&world)).unwrap();
    text.push_str("not json\n");
    fs::write(ledger_of(&world), text).unwrap();
    let out = world.run(&["ledger", "redact", "--ledger-redact-names"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("1 unparseable line(s) were left as-is and may still hold raw ids"),
        "{err}"
    );
    assert!(
        err.contains("already redacted without names are not re-hashed"),
        "{err}"
    );
    let plain = world.run(&["ledger", "redact"]);
    assert!(
        !stderr(&plain).contains("already redacted without names"),
        "{}",
        stderr(&plain)
    );
}

const AGENT_BRANCH: &str = "worktree-agent-a1";

/// Records the agent branch redacted, then adds a copy of every line with the branch respelled
/// raw, as a ledger written before branch hashing would hold it. Returns the respelled text.
fn with_both_branch_spellings(world: &World) -> String {
    let out = world.run(&["record", AGENT_BRANCH, "--strict"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = fs::read_to_string(ledger_of(world)).unwrap();
    let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let hashed = first["recorded_for"].as_str().unwrap().to_owned();
    assert_ne!(hashed, AGENT_BRANCH, "the branch was not hashed");
    text.replace(&hashed, AGENT_BRANCH)
}

#[test]
fn a_call_under_both_branch_spellings_is_counted_once_and_noted() {
    let world = opted_in();
    let respelled = with_both_branch_spellings(&world);
    let mut text = fs::read_to_string(ledger_of(&world)).unwrap();
    text.push_str(&respelled);
    fs::write(ledger_of(&world), text).unwrap();
    let out = world.run(&[
        "report",
        AGENT_BRANCH,
        "--strict",
        "--from-ledger",
        "--allow-ledger-conflicts",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("ledger line(s) held the same call under both the raw and hashed spelling"),
        "{err}"
    );
    assert!(err.contains("counted once"), "{err}");
}

#[test]
fn ledger_redact_counts_lines_it_cannot_repair_without_their_salt() {
    let world = opted_in();
    let respelled = with_both_branch_spellings(&world);
    fs::write(ledger_of(&world), respelled).unwrap();
    fs::write(salt_of(&world), "cd".repeat(32) + "\n").unwrap();
    let out = world.run(&["ledger", "redact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("line(s) hashed under another salt still carry a raw agent branch name"),
        "{err}"
    );
    assert!(err.contains("can't be repaired without that salt"), "{err}");
    assert!(!err.contains(&"cd".repeat(8)), "{err}");
}

#[test]
fn record_with_nothing_to_append_creates_no_salt_and_an_append_persists_it() {
    let world = opted_in();
    let out = world.run(&["record", "no/such-branch"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "appended 0, skipped 0, branch no/such-branch\n"
    );
    assert!(!salt_of(&world).exists(), "a salt was created for nothing");
    assert!(!ledger_of(&world).exists());

    record(&world, &[]);
    assert!(salt_of(&world).is_file());
    let fingerprint = lines(&ledger_of(&world))[0]["redaction"].clone();
    assert!(fingerprint.is_string());
    let all = [stdout(&world.run(&["ledger", "redact"])), String::new()].concat();
    assert!(!all.contains(fingerprint.as_str().unwrap()), "{all}");
}

#[test]
fn config_show_lists_the_ledger_redact_keys_with_their_sources() {
    let world = World::scenario(SCENARIO);
    let defaults = stdout(&world.run(&["config", "show"]));
    assert!(
        defaults.contains("ledger-redact = true (default)"),
        "{defaults}"
    );
    assert!(
        defaults.contains("ledger-redact-names = false (default)"),
        "{defaults}"
    );
    world.write_default_config("ledger-redact = false\nledger-redact-names = true\n");
    let set = stdout(&world.run(&["config", "show"]));
    assert!(set.contains("ledger-redact = false (config file)"), "{set}");
    assert!(
        set.contains("ledger-redact-names = true (config file)"),
        "{set}"
    );
}
