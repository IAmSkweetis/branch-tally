//! End-to-end tests: the built binary against the shared fixtures, in a real git repository.
//!
//! `--pr` talks to `gh`, which is not faked here (a portable fake executable needs a manifest
//! target); its splice logic is tested against the fake `Gh` in core (`tests/pr.rs`).

#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, fixtures_root, stderr, stdout};
use std::fs;

const NO_RECORDS_TAIL: &str =
    ". Transcripts are local and purged after cleanupPeriodDays (default 30).";

fn scenarios_with(golden: &str) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(fixtures_root())
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().join("goldens").join(golden).is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// Runs `report <branch> --json [extra]` for every branch in the scenario's golden and compares
/// the parsed payload, or the no-records exit, with what the Python reference produced.
///
/// `strict` is whether the run is `--strict`. In claim mode the listed `CLAIM_DIVERGENCES` pairs
/// must still differ from the reference and every other pair must equal it (#6).
fn check_against_golden(golden: &str, extra: &[&str], strict: bool) -> usize {
    let mut compared = 0;
    let mut divergences_hit = 0;
    for name in scenarios_with(golden) {
        let world = World::scenario(&name);
        let text =
            fs::read_to_string(fixtures_root().join(&name).join("goldens").join(golden)).unwrap();
        for (branch, want) in serde_json::from_str::<serde_json::Value>(&text)
            .unwrap()
            .as_object()
            .unwrap()
        {
            let mut args = vec!["report", branch.as_str(), "--json"];
            args.extend_from_slice(extra);
            let out = world.run(&args);
            if want.get("error").is_some() {
                assert_eq!(out.status.code(), Some(1), "{name} {branch}");
                assert!(stdout(&out).is_empty(), "{name} {branch}");
                assert_eq!(
                    stderr(&out),
                    format!(
                        "No transcript records found for branch {}{NO_RECORDS_TAIL}\n",
                        py_repr(branch)
                    ),
                    "{name} {branch}"
                );
            } else {
                let divergence = if strict {
                    None
                } else {
                    CLAIM_DIVERGENCES
                        .iter()
                        .find(|(s, b, _, _)| *s == name && *b == branch.as_str())
                };
                if divergence.is_some() && out.status.code() == Some(1) {
                    // The rollup moved every record of this branch to its dispatcher, so claim
                    // mode finds none: the no-records exit differs from the reference's payload.
                    assert!(stdout(&out).is_empty(), "{name} {branch}");
                    assert_eq!(
                        stderr(&out),
                        format!(
                            "No transcript records found for branch {}{NO_RECORDS_TAIL}\n",
                            py_repr(branch)
                        ),
                        "{name} {branch}"
                    );
                    divergences_hit += 1;
                    compared += 1;
                    continue;
                }
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{name} {branch}: {}",
                    stderr(&out)
                );
                let mut got: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
                world.normalize(&mut got);
                strip_price_divergences(&mut got);
                strip_heuristics_divergence(&mut got, strict);
                strip_subagent_divergence(&mut got, strict);
                strip_unstamped_divergence(&mut got, &name, strict);
                match divergence {
                    None => assert_eq!(&got, want, "{name} {branch}"),
                    Some((_, _, issue, _)) => {
                        assert_ne!(
                            &got, want,
                            "{name} {branch}: listed as a divergence ({issue}) but now matches; \
                             remove it"
                        );
                        divergences_hit += 1;
                    }
                }
            }
            compared += 1;
        }
    }
    if !strict {
        assert_eq!(divergences_hit, CLAIM_DIVERGENCES.len());
    }
    compared
}

/// Deliberate divergences of the claim-mode report from the reference (#6, #37), as `(scenario,
/// branch, issue, reason)`. Strict mode stays raw and never diverges. This is a copy of
/// `CLAIM_DIVERGENCES` in `branch-tally-core/tests/parity_claude_code.rs` (that list is private to
/// a test target); keep the two identical.
const CLAIM_DIVERGENCES: &[(&str, &str, &str, &str)] = &[
    (
        "subagent",
        "feat/x",
        "#6",
        "dispatched subagent calls the reference stamped elsewhere roll up to the dispatcher",
    ),
    (
        "subagent-renamed-branch",
        "feat/x",
        "#6",
        "the subagent's calls, including the one stamped after it renamed its branch, roll up \
         to the dispatcher's feat/x",
    ),
    (
        "subagent-renamed-branch",
        "chore/x-part",
        "#6",
        "its only call belongs to a dispatched subagent, so it rolls up to feat/x and the \
         renamed branch has no records of its own in claim mode",
    ),
    (
        "subagent-switches-branch",
        "feat/x",
        "#6",
        "all eight subagent calls roll up to the dispatcher, including those stamped with the \
         subagents' own branches",
    ),
    (
        "subagent-switches-branch",
        "chore/d1-data",
        "#6",
        "the subagent's calls roll up to feat/x, so this branch has no records of its own in \
         claim mode",
    ),
    (
        "subagent-switches-branch",
        "chore/d2-data",
        "#6",
        "the subagent's calls roll up to feat/x, so this branch has no records of its own in \
         claim mode",
    ),
    (
        "unstamped-main",
        "main",
        "#37",
        "the reference recovers unstamped main-checkout calls onto the branch the main checkout \
         has checked out now; they stay (none), and the report counts them in `unstamped`",
    ),
];

/// #7: a claim-mode payload that used an attribution heuristic carries a `heuristics` object the
/// reference does not emit. A report-field divergence, listed here next to the #6 ones and
/// stripped deliberately (after checking its values) so no other key is loosened. Mirrors
/// `without_heuristics_divergence` in `branch-tally-core/tests/parity_claude_code.rs`.
fn strip_heuristics_divergence(payload: &mut serde_json::Value, strict: bool) {
    let object = payload.as_object_mut().unwrap();
    let sub = object.get("subagents").cloned();
    let Some(h) = object.remove("heuristics") else {
        return;
    };
    assert!(!strict, "strict mode must not carry a heuristics object");
    let calls = |v: &serde_json::Value, k: &str| v[k]["calls"].as_u64().unwrap();
    assert_eq!(calls(&h, "timelineForward"), 0);
    assert_eq!(calls(&h, "timelineCarried"), 0);
    assert_eq!(
        h["isFloor"].as_bool().unwrap(),
        calls(&h, "ambiguous") > 0,
        "isFloor disagrees with the ambiguous calls"
    );
    let (sub_calls, sub_tokens) = sub.as_ref().map_or((0, 0), |s| {
        (s["calls"].as_u64().unwrap(), s["tokens"].as_u64().unwrap())
    });
    assert_eq!(calls(&h, "dispatcherRollUp"), sub_calls);
    assert_eq!(
        h["dispatcherRollUp"]["tokens"].as_u64().unwrap(),
        sub_tokens
    );
}

/// Exact claim-mode `unstamped` `(calls, tokens)` per scenario (#60); a scenario not listed must
/// report none. Copy of `CLAIM_UNSTAMPED` in `branch-tally-core/tests/parity_claude_code.rs`.
const CLAIM_UNSTAMPED: &[(&str, (u64, u64))] = &[("unstamped-main", (1, 1_115))];

/// The reference's own `(none)` bucket for a scenario, from its strict golden: `(calls, tokens)` of
/// every call that carries no branch stamp. `(0, 0)` when the golden has no such key.
fn reference_none(scenario: &str) -> (u64, u64) {
    let path = fixtures_root()
        .join(scenario)
        .join("goldens")
        .join("strict.json");
    let golden: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let Some(none) = golden.get("(none)") else {
        return (0, 0);
    };
    let tokens = none["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["tokens"].as_u64().unwrap())
        .sum();
    (none["totalCalls"].as_u64().unwrap(), tokens)
}

/// #60: a payload with unstamped calls carries a top-level `unstamped` object the reference does
/// not emit. A report-field divergence, stripped deliberately after its values are checked so no
/// other key is loosened. Strict mode is raw, so its count is exactly the reference's `(none)`
/// bucket; claim mode is the exact `CLAIM_UNSTAMPED` entry. Mirrors
/// `without_unstamped_divergence` in `branch-tally-core/tests/parity_claude_code.rs`.
fn strip_unstamped_divergence(payload: &mut serde_json::Value, scenario: &str, strict: bool) {
    let (raw_calls, raw_tokens) = reference_none(scenario);
    let got = payload.as_object_mut().unwrap().remove("unstamped");
    let (calls, tokens) = got.as_ref().map_or((0, 0), |u| {
        assert_eq!(u.as_object().unwrap().len(), 2, "unexpected unstamped keys");
        (u["calls"].as_u64().unwrap(), u["tokens"].as_u64().unwrap())
    });
    if got.is_some() {
        assert!(calls > 0, "a zero count must be omitted");
    }
    if strict {
        assert_eq!(
            (calls, tokens),
            (raw_calls, raw_tokens),
            "{scenario}: strict is raw"
        );
    } else {
        let want = CLAIM_UNSTAMPED
            .iter()
            .find(|(s, _)| *s == scenario)
            .map_or((0, 0), |(_, v)| *v);
        assert_eq!((calls, tokens), want, "{scenario}: claim-mode unstamped");
        assert!(
            want.0 <= raw_calls && want.1 <= raw_tokens,
            "{scenario}: expected more unstamped calls than the transcripts hold"
        );
    }
}

/// Checks the #6 `subagents` object, when present, adds up on its own terms and is absent in
/// strict mode, then removes it so the rest of the payload can be compared with the golden.
fn strip_subagent_divergence(payload: &mut serde_json::Value, strict: bool) {
    let Some(sub) = payload.as_object_mut().unwrap().remove("subagents") else {
        return;
    };
    assert!(!strict, "strict mode must not carry a subagents breakdown");
    let rows = sub["rows"].as_array().unwrap();
    assert!(
        !rows.is_empty() || sub["unattributedCalls"].as_u64().unwrap() > 0,
        "an empty breakdown must be omitted"
    );
    for field in ["calls", "tokens"] {
        let sum: u64 = rows.iter().map(|r| r[field].as_u64().unwrap()).sum();
        assert_eq!(
            sub[field].as_u64().unwrap(),
            sum,
            "subagents.{field} != sum of rows"
        );
    }
}

/// Top-level JSON keys the reference does not have (#5): the floor flag, the unpriced model names
/// and the price table's date and source. Each must be present; they are the only keys exempt
/// from the golden comparison.
const PRICE_DIVERGENT_KEYS: &[&str] = &["totalCostIsFloor", "unpricedModels", "priceTable"];

fn strip_price_divergences(payload: &mut serde_json::Value) {
    let object = payload.as_object_mut().unwrap();
    for key in PRICE_DIVERGENT_KEYS {
        assert!(object.remove(*key).is_some(), "{key} missing from --json");
    }
}

fn py_repr(s: &str) -> String {
    // Fixture branches are plain ASCII without quotes.
    format!("'{s}'")
}

#[test]
fn json_claim_matches_the_reference_goldens() {
    assert!(check_against_golden("claim.json", &[], false) >= 27);
}

#[test]
fn json_strict_matches_the_reference_goldens() {
    assert!(check_against_golden("strict.json", &["--strict"], true) >= 27);
}

#[test]
fn json_output_is_pretty_utf8_with_lf_and_one_trailing_newline() {
    let world = World::scenario("subagent");
    let out = world.run(&["report", "feat/x", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
    assert!(!text.contains('\r'));
    assert!(text.starts_with("{\n  \"branch\": \"feat/x\""));
}

#[test]
fn markdown_is_the_default_and_wrapped_in_markers() {
    let world = World::scenario("subagent");
    let out = world.run(&["report", "feat/x"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(!text.contains('\r'));
    assert!(text.starts_with("<!-- token-report:start -->\n"));
    assert!(text.ends_with("\n<!-- token-report:end -->\n"));
    assert_eq!(world.run(&["report", "feat/x", "--md"]).stdout, out.stdout);
    insta::assert_snapshot!("markdown_subagent", text);
}

#[test]
fn markdown_keeps_non_ascii_as_utf8_bytes() {
    let world = World::scenario("worktree-and-siblings");
    let out = world.run(&["report", "wt-a"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // The span arrow is U+2192, three bytes in UTF-8 whatever the console codepage.
    assert!(out.stdout.windows(3).any(|w| w == [0xE2, 0x86, 0x92]));
    insta::assert_snapshot!("markdown_worktree_session", stdout(&out));
}

#[test]
fn no_cost_drops_the_cost_column_in_markdown() {
    let world = World::scenario("subagent");
    let out = world.run(&["report", "feat/x", "--no-cost"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(!text.contains("Est. cost"));
    assert!(!text.contains("est. cost at list API rates"));
    insta::assert_snapshot!("markdown_no_cost", text);
}

#[test]
fn no_cost_does_not_change_json() {
    let world = World::scenario("subagent");
    let plain = world.run(&["report", "feat/x", "--json"]).stdout;
    let no_cost = world
        .run(&["report", "feat/x", "--json", "--no-cost"])
        .stdout;
    assert_eq!(plain, no_cost);
}

#[test]
fn strict_markdown_reports_the_naive_filter() {
    let world = World::scenario("main-then-branch");
    let claim = world.run(&["report", "feat/new"]);
    let strict = world.run(&["report", "feat/new", "--strict"]);
    assert_eq!(strict.status.code(), Some(0), "{}", stderr(&strict));
    assert_ne!(claim.stdout, strict.stdout);
    insta::assert_snapshot!("markdown_strict", stdout(&strict));
}

#[test]
fn branch_defaults_to_the_current_branch() {
    let world = World::scenario("subagent");
    let git = |args: &[&str]| {
        let s = std::process::Command::new("git")
            .args(args)
            .current_dir(&world.repo)
            .status()
            .unwrap();
        assert!(s.success());
    };
    git(&["checkout", "-q", "-b", "feat/x"]);
    let implicit = world.run(&["report", "--json"]);
    let explicit = world.run(&["report", "feat/x", "--json"]);
    assert_eq!(implicit.status.code(), Some(0), "{}", stderr(&implicit));
    assert_eq!(implicit.stdout, explicit.stdout);
}

#[test]
fn repo_flag_selects_the_repository_and_a_worktree_resolves_to_the_main_checkout() {
    let world = World::scenario("subagent");
    let from_elsewhere = std::env::temp_dir();
    let via_flag = world.run_in(
        &from_elsewhere,
        &[
            "report",
            "feat/x",
            "--json",
            "--repo",
            &world.repo.to_string_lossy(),
        ],
    );
    let direct = world.run(&["report", "feat/x", "--json"]);
    assert_eq!(via_flag.status.code(), Some(0), "{}", stderr(&via_flag));
    assert_eq!(via_flag.stdout, direct.stdout);

    // From inside a linked worktree, git-common-dir leads back to the main checkout.
    let wt = world
        .repo
        .join(".claude")
        .join("worktrees")
        .join("agent-a1");
    let from_wt = world.run_in(&wt, &["report", "feat/x", "--json"]);
    assert_eq!(from_wt.status.code(), Some(0), "{}", stderr(&from_wt));
    assert_eq!(from_wt.stdout, direct.stdout);
}

#[test]
fn no_records_exits_1_with_the_reference_message() {
    let world = World::scenario("subagent");
    let out = world.run(&["report", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, Vec::<u8>::new());
    assert_eq!(
        stderr(&out),
        format!("No transcript records found for branch 'nope'{NO_RECORDS_TAIL}\n")
    );
}

#[test]
fn no_records_message_follows_pythons_repr_quoting() {
    let world = World::scenario("subagent");
    let cases = [
        ("it's", "\"it's\""),
        ("a\"b'c", "'a\"b\\'c'"),
        ("back\\slash", "'back\\\\slash'"),
        ("caf\u{e9}", "'caf\u{e9}'"),
    ];
    for (branch, repr) in cases {
        let out = world.run(&["report", branch]);
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(
            stderr(&out),
            format!("No transcript records found for branch {repr}{NO_RECORDS_TAIL}\n"),
            "{branch}"
        );
    }
}

#[test]
fn pr_with_no_records_fails_before_touching_gh() {
    let world = World::scenario("subagent");
    let out = world.run(&["report", "nope", "--pr", "--dry-run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).starts_with("No transcript records found for branch 'nope'"));
}

#[test]
fn missing_projects_directory_exits_1() {
    let world = World::empty();
    fs::remove_dir_all(world.home.join(".claude")).unwrap();
    let out = world.run(&["report", "main"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.starts_with("No Claude Code transcript directory at "),
        "{err}"
    );
    assert!(err.contains("projects"));
    assert!(err.ends_with('\n'));
}

#[test]
fn no_transcript_directory_for_the_repository_exits_1() {
    let world = World::empty();
    let out = world.run(&["report", "main"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.starts_with("No transcript directory for "), "{err}");
    assert!(err.ends_with(")\n"), "{err}");
    // #40: every candidate slug is named, not just the first.
    for slug in branch_tally_core::paths::slug_candidates(&common::canon(&world.repo)) {
        let slug = slug.to_lowercase();
        assert!(err.contains(&slug), "{slug} missing from {err}");
    }
}

#[test]
fn md_and_json_are_mutually_exclusive() {
    let world = World::empty();
    let out = world.run(&["report", "--md", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, Vec::<u8>::new());
    assert!(stderr(&out).contains("cannot be used with"));
}

#[test]
fn unknown_flags_and_the_deferred_csv_flag_are_argument_errors() {
    let world = World::empty();
    for args in [
        &["report", "--bogus"][..],
        &["report", "--csv"][..],
        &["bogus"][..],
        &[][..],
    ] {
        let out = world.run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(out.stdout, Vec::<u8>::new(), "{args:?}");
        assert!(!stderr(&out).is_empty(), "{args:?}");
    }
}

#[test]
fn help_exits_0_and_lists_the_flags_without_the_projects_override() {
    let world = World::empty();
    let out = world.run(&["report", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    for flag in [
        "--repo",
        "--md",
        "--json",
        "--strict",
        "--no-cost",
        "--pr",
        "--dry-run",
    ] {
        assert!(text.contains(flag), "{flag}");
    }
    assert!(!text.contains("--csv"));
}

/// Stderr without the unpriced-model warnings (#5), which `tests/prices.rs` pins. These fixtures
/// use a model id no price table knows, so every run carries one.
fn notes(out: &std::process::Output) -> String {
    stderr(out)
        .lines()
        .filter(|l| !l.starts_with("warning: no price for "))
        .fold(String::new(), |mut acc, l| {
            acc.push_str(l);
            acc.push('\n');
            acc
        })
}

const SKIP_NOTE: &str =
    "note: skipped 3 transcript record(s): 2 malformed, 1 without a request id\n";

#[test]
fn skipped_records_are_noted_on_stderr_and_stdout_is_untouched() {
    let world = World::scenario("malformed-and-keyless");
    let out = world.run(&["report", "main"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(notes(&out), SKIP_NOTE);
    assert!(stdout(&out).starts_with("<!-- token-report:start -->\n"));
    let json = world.run(&["report", "main", "--json"]);
    assert_eq!(notes(&json), SKIP_NOTE);
    assert!(stdout(&json).starts_with("{\n"));
}

#[test]
fn skipped_records_are_noted_before_the_no_records_error() {
    let world = World::scenario("malformed-and-keyless");
    let out = world.run(&["report", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        notes(&out),
        format!("{SKIP_NOTE}No transcript records found for branch 'nope'{NO_RECORDS_TAIL}\n")
    );
}

#[test]
fn synthetic_records_are_counted_in_the_note() {
    let world = World::scenario("synthetic");
    let out = world.run(&["report", "main"]);
    assert_eq!(
        notes(&out),
        "note: skipped 1 transcript record(s): 1 synthetic\n"
    );
}

#[test]
fn replays_and_clean_reads_print_no_note() {
    // Replayed history is deduplicated exactly as the reference does; resumed sessions make it
    // routine, so it is not announced as a skip.
    for (scenario, branch) in [("replayed-history", "main"), ("subagent", "feat/x")] {
        let world = World::scenario(scenario);
        let out = world.run(&["report", branch, "--json"]);
        assert_eq!(out.status.code(), Some(0), "{scenario}");
        assert_eq!(notes(&out), "", "{scenario}");
    }
}

#[test]
fn priced_models_markdown_shows_costs() {
    let world = World::scenario("priced-models");
    let out = world.run(&["report", "main"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Est. cost"));
    insta::assert_snapshot!("markdown_priced_models", text);
}

/// The `subagent-removed-worktree` scenario's subagents ran in worktrees that no longer exist, so
/// only the dispatch links tie their calls to `feat/x`. Dropping the links in the binary's source
/// wrapper (#6) loses every one of them.
#[test]
fn removed_worktree_subagents_roll_up_to_the_dispatcher_through_the_binary() {
    let world = World::scenario("subagent-removed-worktree");
    let out = world.run(&["report", "feat/x", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(json["totalCalls"], 7);
    let input: u64 = json["models"]
        .as_object()
        .unwrap()
        .values()
        .map(|m| m["input"].as_u64().unwrap())
        .sum();
    assert_eq!(input, 28);
    let sub = &json["subagents"];
    assert_eq!(sub["calls"], 5);
    assert_eq!(sub["unattributedCalls"], 0);
    assert_eq!(sub["unlinkedInTotalCalls"], 0);
}

/// Each subagent row's usage split must add up to its Tokens cell, and the rows must add up to
/// the section (#6).
#[test]
fn subagent_rows_usage_split_sums_to_their_tokens_through_the_binary() {
    for scenario in ["subagent", "subagent-removed-worktree"] {
        let world = World::scenario(scenario);
        let out = world.run(&["report", "feat/x", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        let sub = &json["subagents"];
        let rows = sub["rows"].as_array().unwrap();
        assert!(!rows.is_empty(), "{scenario}: no subagent rows");
        let (mut calls, mut tokens) = (0, 0);
        let mut keys = std::collections::HashSet::new();
        for row in rows {
            let model = row["model"].as_str().unwrap_or("");
            assert!(!model.is_empty(), "{scenario}: row without a model");
            let key = (
                row["workspace"].to_string(),
                row["workspaceKind"].to_string(),
                row["subagent"].to_string(),
                model.to_string(),
            );
            assert!(keys.insert(key), "{scenario}: duplicate subagent/model row");
            let split: u64 = [
                "input",
                "cacheWrite5m",
                "cacheWrite1h",
                "cacheRead",
                "output",
            ]
            .iter()
            .map(|k| row[*k].as_u64().expect("usage field is a number"))
            .sum();
            assert_eq!(split, row["tokens"].as_u64().unwrap(), "{scenario}");
            assert!(row["thinking"].is_u64(), "{scenario}");
            assert!(row["costIsFloor"].is_boolean(), "{scenario}");
            calls += row["calls"].as_u64().unwrap();
            tokens += row["tokens"].as_u64().unwrap();
        }
        assert_eq!(calls, sub["calls"].as_u64().unwrap(), "{scenario}");
        assert_eq!(tokens, sub["tokens"].as_u64().unwrap(), "{scenario}");
        assert!(sub["costIsFloor"].is_boolean(), "{scenario}");
    }
}

#[test]
fn strict_never_shows_the_subagent_usage_section() {
    for scenario in ["subagent", "subagent-removed-worktree"] {
        let world = World::scenario(scenario);
        let claim = world.run(&["report", "feat/x"]);
        assert!(stdout(&claim).contains("### Subagent usage"), "{scenario}");
        let strict = world.run(&["report", "feat/x", "--strict"]);
        assert_eq!(strict.status.code(), Some(0), "{}", stderr(&strict));
        assert!(!stdout(&strict).contains("Subagent usage"), "{scenario}");
        let json = world.run(&["report", "feat/x", "--strict", "--json"]);
        assert!(!stdout(&json).contains("\"subagents\""), "{scenario}");
    }
}

/// Rewrites the priced-models transcript in the world's projects directory: the unpriced model id
/// gets terminal and bidi controls, and one record's timestamp stops being RFC 3339.
fn hostile_world() -> World {
    let world = World::scenario("priced-models");
    let mut rewritten = 0;
    for dir in fs::read_dir(world.projects()).unwrap() {
        let file = dir.unwrap().path().join("s1.jsonl");
        if !file.is_file() {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        let text = text
            .replace("claude-mystery-9", "claude-mystery-\\u001b[31m\\u202e9")
            .replace("2026-01-01T00:00:04.000Z", "not-a-timestamp");
        fs::write(&file, text).unwrap();
        rewritten += 1;
    }
    assert_eq!(rewritten, 1);
    world
}

#[test]
fn control_characters_in_recorded_text_never_reach_stderr() {
    let out = hostile_world().run(&["report", "main", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let err = stderr(&out);
    assert!(
        err.contains("warning: no price for claude-mystery-"),
        "{err}"
    );
    for bad in ['\u{1b}', '\u{202e}'] {
        assert!(!err.contains(bad), "{err:?}");
    }
    assert!(err.contains('\u{fffd}'), "{err:?}");
}

#[test]
fn malformed_timestamps_are_counted_in_a_stderr_note_with_no_content() {
    let out = hostile_world().run(&["report", "main", "--json"]);
    let err = stderr(&out);
    assert!(
        err.contains(
            "note: 1 record(s) had an unreadable timestamp; they were kept but are undated\n"
        ),
        "{err}"
    );
    assert!(!err.contains("not-a-timestamp"), "{err}");
    // A clean run says nothing about timestamps.
    let clean = World::scenario("priced-models").run(&["report", "main", "--json"]);
    assert!(!stderr(&clean).contains("timestamp"));
}
