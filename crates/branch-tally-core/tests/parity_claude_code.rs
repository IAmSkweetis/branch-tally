//! Parity of the Claude Code source with `reference/token_report.py`, through the goldens that
//! `just goldens` writes next to each scenario (`tests/fixtures/claude-code/<scenario>/goldens/`).
//!
//! Phase 0 compares only what the source can already produce, against `strict.json` (the
//! reference run with `--strict --json`, one key per raw `gitBranch`, `"(none)"` when absent):
//! calls kept and token totals, per session and per model. Cost, origin, first/last and agents
//! belong to the report test below, which compares the full JSON report (claim and strict).
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

use branch_tally_core::git::Worktree;
use branch_tally_core::model::Record;
use branch_tally_core::render;
use branch_tally_core::report::{self, ReportError};
use branch_tally_core::sources::claude_code::ClaudeCode;
use branch_tally_core::sources::{ReadStats, RepoIdentity, Source};
use serde_json::Value;

/// Scenarios the reference cannot be run on. Each must carry a matching `reference_divergence`
/// in its manifest, and must have no goldens.
const REFERENCE_DIVERGENT_SCENARIOS: &[(&str, &str)] = &[("malformed-and-keyless", "#3")];

/// Deliberate divergences between Rust and the reference, as `(scenario, branch key, issue,
/// reason)`. A listed key is asserted to differ, so the entry cannot outlive its reason.
const DIVERGENCES: &[(&str, &str, &str, &str)] = &[];

/// Scenarios and branch keys compared. Pinned so the test cannot green by comparing nothing.
/// Claim pairs the report test compares.
const EXPECTED_CLAIM_PAIRS: usize = 27;
const EXPECTED_SCENARIOS: usize = 14;
const EXPECTED_BRANCH_KEYS: usize = 35;

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

/// Worktrees as `git worktree list` reports them for the generator's throwaway repo: the main
/// checkout on `main` (which the reference also maps; #37), then each manifest worktree under
/// `.claude/worktrees/<name>`.
fn worktrees_for(manifest: &Value) -> Vec<Worktree> {
    let repo = PathBuf::from(r"C:\repo");
    let mut list = vec![Worktree {
        path: repo.clone(),
        branch: "main".to_owned(),
    }];
    for wt in manifest["worktrees"].as_array().unwrap() {
        list.push(Worktree {
            path: repo
                .join(".claude")
                .join("worktrees")
                .join(wt["name"].as_str().unwrap()),
            branch: wt["branch"].as_str().unwrap().to_owned(),
        });
    }
    list
}

/// Top-level JSON keys Rust adds that the reference does not have (#5): the floor flag, the
/// names of unpriced models and the price table's date and source. They are the only keys exempt
/// from the comparison; every other key must equal the golden exactly.
const PRICE_DIVERGENT_KEYS: &[&str] = &["totalCostIsFloor", "unpricedModels", "priceTable"];

/// Checks the #5 keys are present and agree with the report's own per-model costs, then removes
/// them so the rest of the payload can be compared with the golden.
fn without_price_divergences(mut payload: Value) -> Value {
    let object = payload.as_object_mut().unwrap();
    let unpriced: Vec<Value> = object["models"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, m)| m["costUSD"].is_null())
        .map(|(name, _)| Value::String(name.clone()))
        .collect();
    assert_eq!(
        object["totalCostIsFloor"],
        Value::Bool(!unpriced.is_empty()),
        "floor flag disagrees with the unpriced models"
    );
    let mut named: Vec<Value> = object["unpricedModels"].as_array().unwrap().clone();
    named.sort_by_key(ToString::to_string);
    let mut want = unpriced;
    want.sort_by_key(ToString::to_string);
    assert_eq!(named, want, "unpricedModels disagrees with the models");
    for key in PRICE_DIVERGENT_KEYS {
        assert!(
            object.remove(*key).is_some(),
            "{key} missing from the report"
        );
    }
    payload
}

/// Rust's report for `branch` as the reference's `--json` payload, or `{"error":"no_records"}`.
fn rust_payload(dir: &Path, manifest: &Value, branch: &str, strict: bool) -> Value {
    match report::build(
        read_records(dir),
        ReadStats::default(),
        Path::new(r"C:\repo"),
        &worktrees_for(manifest),
        branch,
        strict,
        branch_tally_core::pricing::PriceTable::bundled(),
    ) {
        Ok(r) => without_price_divergences(
            serde_json::from_str(&render::json(&r.summary, &r.costs)).unwrap(),
        ),
        Err(ReportError::NoRecords { .. }) => serde_json::json!({"error": "no_records"}),
        Err(e) => panic!("{branch}: {e}"),
    }
}

/// The reference reports a session origin as the lowercased directory name it found;
/// `make_goldens.py` maps that back to the fixture's own spelling so the goldens read naturally.
/// That is a generator convenience, not behaviour, so compare origins in the reference's form.
/// (Rust lowercases worktree origins too; see `Origin::label`.)
fn reference_origins(golden: &Value) -> Value {
    let mut v = golden.clone();
    if let Some(sessions) = v.get_mut("sessions").and_then(Value::as_array_mut) {
        for s in sessions {
            if let Some(o) = s["origin"].as_str() {
                s["origin"] = Value::String(o.to_lowercase());
            }
        }
    }
    v
}

/// The full report comparison: Rust's report for each `claim.json` key (claim mode) and each
/// `strict.json` key (strict mode) against the reference's payload, as parsed JSON.
#[test]
fn rust_report_matches_the_reference_claim_goldens() {
    let mut claim_pairs = 0;
    let mut strict_pairs = 0;
    let mut failures = Vec::new();
    for (name, dir) in scenarios() {
        let claim_path = dir.join("goldens").join("claim.json");
        if !claim_path.is_file() {
            continue;
        }
        let manifest = read_json(&dir.join("goldens").join("scenario.json"));
        let claim = read_json(&claim_path);
        for branch in manifest["claim_branches"].as_array().unwrap() {
            let branch = branch.as_str().unwrap();
            let golden = claim
                .get(branch)
                .unwrap_or_else(|| panic!("{name}: claim.json has no key {branch}"));
            claim_pairs += 1;
            if rust_payload(&dir, &manifest, branch, false) != reference_origins(golden) {
                failures.push(format!("{name} [{branch}] claim"));
            }
        }
        let strict = read_json(&dir.join("goldens").join("strict.json"));
        for (branch, golden) in strict.as_object().unwrap() {
            strict_pairs += 1;
            if rust_payload(&dir, &manifest, branch, true) != reference_origins(golden) {
                failures.push(format!("{name} [{branch}] strict"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "report differs from goldens: {failures:?}"
    );
    assert_eq!(claim_pairs, EXPECTED_CLAIM_PAIRS, "claim pairs compared");
    assert_eq!(strict_pairs, EXPECTED_BRANCH_KEYS, "strict pairs compared");
}
