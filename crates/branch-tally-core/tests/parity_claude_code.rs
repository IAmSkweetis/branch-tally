//! Parity of the Claude Code source with `reference/token_report.py`, through the goldens that
//! `just goldens` writes next to each scenario (`tests/fixtures/claude-code/<scenario>/goldens/`).
//!
//! Phase 0 compares only what the source can already produce, against `strict.json` (the
//! reference run with `--strict --json`, one key per raw `gitBranch`, `"(none)"` when absent):
//! calls kept and token totals, per session and per model. Cost, origin, first/last and agents
//! belong to later stages. The full report comparison against `claim.json` waits for #4.
//!
//! Scenarios are discovered from disk, so a new scenario with goldens is covered automatically.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail loudly on bad fixtures"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use branch_tally_core::model::Record;
use branch_tally_core::sources::claude_code::ClaudeCode;
use branch_tally_core::sources::{RepoIdentity, Source};
use serde_json::Value;

/// Scenarios the reference cannot be run on. Each must carry a matching `reference_divergence`
/// in its manifest, and must have no goldens.
const REFERENCE_DIVERGENT_SCENARIOS: &[(&str, &str)] = &[("malformed-and-keyless", "#3")];

/// Deliberate divergences between Rust and the reference, as `(scenario, branch key, issue,
/// reason)`. A listed key is asserted to differ, so the entry cannot outlive its reason.
const DIVERGENCES: &[(&str, &str, &str, &str)] = &[];

/// Scenarios and branch keys compared. Pinned so the test cannot green by comparing nothing.
const EXPECTED_SCENARIOS: usize = 12;
const EXPECTED_BRANCH_KEYS: usize = 29;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("claude-code")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every scenario directory that has a manifest, sorted by name.
fn scenarios() -> Vec<(String, PathBuf)> {
    let mut found: Vec<_> = fs::read_dir(fixtures_root())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("goldens").join("scenario.json").is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    found.sort();
    found
}

fn read_records(dir: &Path) -> Vec<Record> {
    let roots = ClaudeCode
        .discover(dir, &RepoIdentity::new(r"C:\repo"))
        .unwrap();
    let mut records = Vec::new();
    ClaudeCode.read(&roots, &mut |r| records.push(r)).unwrap();
    records
}

/// The reference's `Record.tokens`: input + 5m write + 1h write + cache read + output.
/// Thinking is part of output, not added again.
fn tokens(r: &Record) -> u64 {
    let u = &r.usage;
    u.input + u.cache_write_5m + u.cache_write_1h + u.cache_read + u.output
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ModelTotals {
    calls: u64,
    input: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
    output: u64,
    thinking: u64,
}

/// The comparable shape of one branch key: total calls, `session -> (calls, tokens)` and
/// `model -> totals`.
#[derive(Default, Debug, PartialEq, Eq)]
struct Shape {
    total_calls: u64,
    sessions: BTreeMap<String, (u64, u64)>,
    models: BTreeMap<String, ModelTotals>,
}

fn rust_shape(records: &[&Record]) -> Shape {
    let mut s = Shape::default();
    for r in records {
        s.total_calls += 1;
        let e = s
            .sessions
            .entry(r.session_id.as_str().to_owned())
            .or_default();
        e.0 += 1;
        e.1 += tokens(r);
        // The reference names a record with no model "unknown".
        let model = r.model.clone().unwrap_or_else(|| "unknown".to_owned());
        let m = s.models.entry(model).or_default();
        m.calls += 1;
        m.input += r.usage.input;
        m.cache_write_5m += r.usage.cache_write_5m;
        m.cache_write_1h += r.usage.cache_write_1h;
        m.cache_read += r.usage.cache_read;
        m.output += r.usage.output;
        m.thinking += r.usage.thinking.unwrap_or(0);
    }
    s
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("golden field {key} missing or not a number"))
}

fn golden_shape(payload: &Value) -> Shape {
    let mut s = Shape {
        total_calls: num(payload, "totalCalls"),
        ..Shape::default()
    };
    for session in payload["sessions"].as_array().unwrap() {
        let id = session["id"].as_str().unwrap().to_owned();
        assert!(
            s.sessions
                .insert(id, (num(session, "calls"), num(session, "tokens")))
                .is_none(),
            "duplicate session id in a golden"
        );
    }
    for (name, m) in payload["models"].as_object().unwrap() {
        s.models.insert(
            name.clone(),
            ModelTotals {
                calls: num(m, "calls"),
                input: num(m, "input"),
                cache_write_5m: num(m, "cacheWrite5m"),
                cache_write_1h: num(m, "cacheWrite1h"),
                cache_read: num(m, "cacheRead"),
                output: num(m, "output"),
                thinking: num(m, "thinking"),
            },
        );
    }
    s
}

#[test]
fn rust_matches_the_reference_on_calls_and_tokens() {
    let mut scenarios_compared = 0;
    let mut keys_compared = 0;
    let mut divergences_hit = Vec::new();
    let mut failures = Vec::new();

    for (name, dir) in scenarios() {
        let manifest = read_json(&dir.join("goldens").join("scenario.json"));
        let strict_path = dir.join("goldens").join("strict.json");
        let listed = REFERENCE_DIVERGENT_SCENARIOS
            .iter()
            .find(|(s, _)| *s == name);
        if let Some((_, issue)) = listed {
            assert_eq!(
                manifest["reference_divergence"]["issue"].as_str(),
                Some(*issue),
                "{name}: manifest and test disagree about the reference divergence"
            );
            assert!(
                !strict_path.exists(),
                "{name}: divergent scenario has goldens"
            );
            continue;
        }
        assert!(
            manifest.get("reference_divergence").is_none(),
            "{name}: manifest declares a reference divergence the test does not list"
        );
        assert!(
            strict_path.is_file(),
            "{name}: goldens/strict.json missing; run `just goldens {name}`"
        );

        let records = read_records(&dir);
        let mut by_branch: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
        for r in &records {
            let key = r.git_branch.clone().unwrap_or_else(|| "(none)".to_owned());
            by_branch.entry(key).or_default().push(r);
        }

        let golden = read_json(&strict_path);
        let golden = golden.as_object().unwrap();
        scenarios_compared += 1;

        // A key on only one side is a record one of them dropped or invented.
        let mut keys: Vec<&String> = golden.keys().chain(by_branch.keys()).collect();
        keys.sort();
        keys.dedup();

        for key in keys {
            keys_compared += 1;
            let rust = rust_shape(by_branch.get(key).map_or(&[][..], Vec::as_slice));
            let reference = match golden.get(key) {
                None => {
                    failures.push(format!(
                        "{name} [{key}]: Rust has records, golden has no key"
                    ));
                    continue;
                }
                Some(g) if g.get("error").and_then(Value::as_str) == Some("no_records") => {
                    Shape::default()
                }
                Some(g) => golden_shape(g),
            };
            let divergence = DIVERGENCES
                .iter()
                .find(|(s, k, _, _)| *s == name && *k == key.as_str());
            match (divergence, rust == reference) {
                (None, true) => {}
                (None, false) => failures.push(format!(
                    "{name} [{key}]:\n  rust      {rust:?}\n  reference {reference:?}"
                )),
                (Some((_, _, issue, why)), false) => {
                    divergences_hit.push(format!("{name} [{key}] {issue}: {why}"));
                }
                (Some((_, _, issue, _)), true) => failures.push(format!(
                    "{name} [{key}]: listed as a divergence ({issue}) but now matches; remove it"
                )),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "parity failures:\n{}",
        failures.join("\n")
    );
    assert_eq!(
        divergences_hit.len(),
        DIVERGENCES.len(),
        "{divergences_hit:?}"
    );
    assert!(scenarios_compared > 0, "compared zero scenarios");
    assert_eq!(scenarios_compared, EXPECTED_SCENARIOS, "scenarios compared");
    assert_eq!(keys_compared, EXPECTED_BRANCH_KEYS, "branch keys compared");
}

/// The full report comparison: Rust's claim-mode report for each manifest `claim_branches` entry
/// against `claim.json`. It needs attribution, which does not exist yet.
#[test]
#[ignore = "#4: claim-mode attribution and the report do not exist yet"]
fn rust_report_matches_the_reference_claim_goldens() {
    let mut pairs = 0;
    for (name, dir) in scenarios() {
        let claim_path = dir.join("goldens").join("claim.json");
        if !claim_path.is_file() {
            continue;
        }
        let manifest = read_json(&dir.join("goldens").join("scenario.json"));
        let claim = read_json(&claim_path);
        let claim = claim.as_object().unwrap();
        for branch in manifest["claim_branches"].as_array().unwrap() {
            let branch = branch.as_str().unwrap();
            let golden = claim
                .get(branch)
                .unwrap_or_else(|| panic!("{name}: claim.json has no key {branch}"));
            if golden.get("error").is_none() {
                // Parses as a full report payload.
                let _ = golden_shape(golden);
                assert!(golden["totalCostUSD"].is_number(), "{name} [{branch}]");
            }
            pairs += 1;
        }
    }
    assert!(pairs > 0, "no claim goldens found");
    panic!(
        "#4: claim-mode attribution does not exist yet; {pairs} (scenario, branch) claim goldens \
         parse but cannot be compared to a Rust report"
    );
}
