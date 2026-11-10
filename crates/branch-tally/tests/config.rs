//! End-to-end tests for the user config file (#28): the real binary, shared fixtures, and a
//! config directory under the test's own home (never the developer's).
//!
//! Precedence is flag, then config file, then default.

#![expect(clippy::expect_used, reason = "test helpers fail loudly")]

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, stderr, stdout};
use serde_json::Value;

const BRANCH: &str = "feat/new";

fn json(out: &std::process::Output) -> Value {
    assert_eq!(out.status.code(), Some(0), "{}", stderr(out));
    serde_json::from_str(&stdout(out)).expect("stdout is JSON")
}

fn world() -> World {
    World::scenario("main-then-branch")
}

// ---- new flags -------------------------------------------------------------------------------

#[test]
fn mode_flag_matches_the_old_aliases() {
    let w = world();
    for (new, old) in [("strict", "--strict"), ("timeline", "--timeline")] {
        let by_mode = w.run(&["report", BRANCH, "--json", "--mode", new]);
        let by_alias = w.run(&["report", BRANCH, "--json", old]);
        assert_eq!(by_mode.status.code(), Some(0), "{}", stderr(&by_mode));
        assert_eq!(by_mode.stdout, by_alias.stdout, "--mode {new}");
    }
    let claim = w.run(&["report", BRANCH, "--json", "--mode", "claim"]);
    assert_eq!(claim.status.code(), Some(0), "{}", stderr(&claim));
    assert_eq!(claim.stdout, w.run(&["report", BRANCH, "--json"]).stdout);
}

#[test]
fn format_flag_matches_the_old_aliases() {
    let w = world();
    let by_format = w.run(&["report", BRANCH, "--format", "json"]);
    assert_eq!(by_format.status.code(), Some(0), "{}", stderr(&by_format));
    assert_eq!(
        by_format.stdout,
        w.run(&["report", BRANCH, "--json"]).stdout
    );
    let md = w.run(&["report", BRANCH, "--format", "md"]);
    assert_eq!(md.stdout, w.run(&["report", BRANCH]).stdout);
}

#[test]
fn cost_flag_is_accepted_and_is_the_default() {
    let w = world();
    let out = w.run(&["report", BRANCH, "--cost"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(out.stdout, w.run(&["report", BRANCH]).stdout);
    assert!(stdout(&out).contains("Est. cost"));
}

#[test]
fn old_and_new_forms_never_combine_even_when_they_agree() {
    let w = world();
    for args in [
        &["--json", "--format", "md"][..],
        &["--json", "--format", "json"][..],
        &["--md", "--format", "json"][..],
        &["--strict", "--mode", "timeline"][..],
        &["--strict", "--mode", "strict"][..],
        &["--timeline", "--mode", "claim"][..],
        &["--cost", "--no-cost"][..],
    ] {
        let mut full = vec!["report", BRANCH];
        full.extend_from_slice(args);
        let out = w.run(&full);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert_eq!(stdout(&out), "", "{args:?}");
        assert!(stderr(&out).contains("cannot be used with"), "{args:?}");
    }
}

#[test]
fn bad_mode_and_format_values_are_argument_errors() {
    let w = world();
    for args in [&["--mode", "fast"][..], &["--format", "csv"][..]] {
        let mut full = vec!["report", BRANCH];
        full.extend_from_slice(args);
        assert_eq!(w.run(&full).status.code(), Some(2), "{args:?}");
    }
}

// ---- precedence ------------------------------------------------------------------------------

#[test]
fn config_mode_beats_the_default_and_a_flag_beats_config() {
    let w = world();
    w.write_default_config("mode = \"timeline\"\n");
    let from_config = json(&w.run(&["report", BRANCH, "--json"]));
    assert!(
        from_config.get("workBranch").is_some(),
        "timeline from config"
    );

    let by_mode_flag = json(&w.run(&["report", BRANCH, "--json", "--mode", "claim"]));
    assert!(by_mode_flag.get("workBranch").is_none());
    assert_eq!(by_mode_flag["mode"], "claim");

    let by_alias = json(&w.run(&["report", BRANCH, "--json", "--strict"]));
    assert_eq!(by_alias["mode"], "strict");
}

#[test]
fn config_format_beats_the_default_and_a_flag_beats_config() {
    let w = world();
    w.write_default_config("format = \"json\"\n");
    json(&w.run(&["report", BRANCH]));
    let md = w.run(&["report", BRANCH, "--format", "md"]);
    assert!(stdout(&md).contains("Est. cost"), "{}", stdout(&md));
    let md_alias = w.run(&["report", BRANCH, "--md"]);
    assert_eq!(md.stdout, md_alias.stdout);
}

#[test]
fn config_cost_false_hides_cost_and_cost_flag_restores_it() {
    let w = world();
    w.write_default_config("cost = false\n");
    assert!(!stdout(&w.run(&["report", BRANCH])).contains("Est. cost"));
    let restored = w.run(&["report", BRANCH, "--cost"]);
    assert!(stdout(&restored).contains("Est. cost"));

    w.write_default_config("cost = true\n");
    let off = w.run(&["report", BRANCH, "--no-cost"]);
    assert!(!stdout(&off).contains("Est. cost"));
}

#[test]
fn config_work_branch_applies_in_timeline_mode_and_a_flag_beats_it() {
    let w = world();
    w.write_default_config("mode = \"timeline\"\nwork-branch = \"^feat/\"\n");
    let v = json(&w.run(&["report", BRANCH, "--json"]));
    assert_eq!(v["workBranch"]["kind"], "custom");
    assert_eq!(v["workBranch"]["pattern"], "^feat/");

    let v = json(&w.run(&["report", BRANCH, "--json", "--work-branch", "new"]));
    assert_eq!(v["workBranch"]["pattern"], "new");
}

#[test]
fn config_work_branch_is_ignored_outside_timeline_mode() {
    let w = world();
    w.write_default_config("work-branch = \"^feat/\"\n");
    let v = json(&w.run(&["report", BRANCH, "--json"]));
    assert!(v.get("workBranch").is_none());
    // Still not an error when the mode is chosen by a flag.
    let v = json(&w.run(&["report", BRANCH, "--json", "--strict"]));
    assert!(v.get("workBranch").is_none());
}

#[test]
fn work_branch_flag_needs_timeline_to_be_the_effective_mode() {
    let w = world();
    // No mode anywhere: an argument error that names the ways to get there.
    let out = w.run(&["report", BRANCH, "--work-branch", "^feat/"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("--timeline"), "{}", stderr(&out));

    // `--mode timeline` is enough.
    let ok = w.run(&[
        "report",
        BRANCH,
        "--mode",
        "timeline",
        "--work-branch",
        "^feat/",
    ]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));

    // So is a config `mode = "timeline"`.
    w.write_default_config("mode = \"timeline\"\n");
    let ok = w.run(&["report", BRANCH, "--work-branch", "^feat/"]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));

    // A flag mode that is not timeline overrides the config, so the flag is an error again.
    let out = w.run(&["report", BRANCH, "--strict", "--work-branch", "^feat/"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

// ---- --config and location -------------------------------------------------------------------

#[test]
fn explicit_config_replaces_the_default_file() {
    let w = world();
    w.write_default_config("format = \"json\"\n");
    let explicit = w.write_config_file("other.toml", "cost = false\n");
    let out = w.run(&["report", BRANCH, "--config", explicit.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(!text.contains("Est. cost"), "explicit file applied");
    assert!(!text.trim_start().starts_with('{'), "default file not read");
}

#[test]
fn missing_default_config_is_normal() {
    let w = world();
    let out = w.run(&["report", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn missing_explicit_config_is_an_error_naming_the_file() {
    let w = world();
    let missing = w.home.join("nope.toml");
    let out = w.run(&["report", BRANCH, "--config", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("nope.toml"), "{}", stderr(&out));
}

// ---- bad config ------------------------------------------------------------------------------

fn bad_config(text: &str) -> String {
    let w = world();
    w.write_default_config(text);
    let out = w.run(&["report", BRANCH]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains("config.toml"), "names the file: {err}");
    assert!(!err.contains("panicked"), "{err}");
    err
}

#[test]
fn malformed_config_exits_1_naming_the_file() {
    let err = bad_config("this is = = not toml\n");
    assert!(err.contains("not valid TOML"), "{err}");
}

#[test]
fn unknown_key_exits_1_naming_the_file_and_key() {
    let err = bad_config("harness = \"claude-code\"\n");
    assert!(err.contains("`harness`"), "{err}");
}

#[test]
fn bad_value_exits_1_naming_the_key() {
    let err = bad_config("mode = \"fast\"\n");
    assert!(err.contains("`mode`"), "{err}");
}

#[test]
fn bad_work_branch_regex_exits_1_naming_the_file_and_key_without_echoing_it() {
    let err = bad_config("mode = \"timeline\"\nwork-branch = \"(secret-pattern\"\n");
    assert!(err.contains("`work-branch`"), "{err}");
    assert!(!err.contains("secret-pattern"), "{err}");
}

#[test]
fn bad_config_fails_even_when_every_flag_is_given() {
    let w = world();
    w.write_default_config("nonsense = 1\n");
    let out = w.run(&["report", BRANCH, "--strict", "--json", "--no-cost"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
}

// ---- config show -----------------------------------------------------------------------------

#[test]
fn config_show_without_a_file_reports_defaults() {
    let w = world();
    let out = w.run(&["config", "show"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("config.toml"), "{text}");
    assert!(text.contains("found: no"), "{text}");
    assert!(text.contains("mode = claim (default)"), "{text}");
    assert!(text.contains("work-branch = (not set) (default)"), "{text}");
    assert!(text.contains("format = md (default)"), "{text}");
    assert!(text.contains("cost = true (default)"), "{text}");
    assert!(!text.contains('\r'), "LF only");
}

#[test]
fn config_show_reports_file_values_and_their_source() {
    let w = world();
    w.write_default_config("mode = \"timeline\"\nwork-branch = \"^feat/\"\ncost = false\n");
    let out = w.run(&["config", "show"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("found: yes"), "{text}");
    assert!(text.contains("mode = timeline (config file)"), "{text}");
    assert!(
        text.contains("work-branch = ^feat/ (config file)"),
        "{text}"
    );
    assert!(!text.contains("inactive"), "{text}");
    assert!(text.contains("format = md (default)"), "{text}");
    assert!(text.contains("cost = false (config file)"), "{text}");
}

#[test]
fn config_show_marks_an_inactive_work_branch() {
    let w = world();
    w.write_default_config("work-branch = \"^feat/\"\n");
    let text = stdout(&w.run(&["config", "show"]));
    assert!(
        text.contains("work-branch = ^feat/ (config file) [inactive: mode is not timeline]"),
        "{text}"
    );
}

#[test]
fn config_show_honours_config_and_rejects_a_missing_explicit_file() {
    let w = world();
    let explicit = w.write_config_file("x.toml", "format = \"json\"\n");
    let out = w.run(&["config", "show", "--config", explicit.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("x.toml"), "{text}");
    assert!(text.contains("format = json (config file)"), "{text}");

    let missing = w.home.join("nope.toml");
    let out = w.run(&["config", "show", "--config", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("nope.toml"));
}

#[test]
fn config_show_fails_on_a_bad_file() {
    let w = world();
    w.write_default_config("bogus = 1\n");
    let out = w.run(&["config", "show"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("`bogus`"), "{}", stderr(&out));
}
