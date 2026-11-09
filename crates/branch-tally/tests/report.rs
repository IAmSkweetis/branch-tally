//! End-to-end tests: the built binary against the shared fixtures, in a real git repository.
//!
//! `--pr` talks to `gh`, which is not faked here (a portable fake executable needs a manifest
//! target); its splice logic is tested against the fake `Gh` in core (`tests/pr.rs`).

#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

mod common;

use common::{World, fixtures_root, parse_json, stderr, stdout};
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
fn check_against_golden(golden: &str, extra: &[&str]) -> usize {
    let mut compared = 0;
    for name in scenarios_with(golden) {
        let world = World::scenario(&name);
        let text =
            fs::read_to_string(fixtures_root().join(&name).join("goldens").join(golden)).unwrap();
        for (branch, want) in parse_json(&text).entries() {
            let mut args = vec!["report", branch.as_str(), "--json"];
            args.extend_from_slice(extra);
            let out = world.run(&args);
            if want.entries().contains_key("error") {
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
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{name} {branch}: {}",
                    stderr(&out)
                );
                let mut got = parse_json(&stdout(&out));
                world.normalize(&mut got);
                assert_eq!(&got, want, "{name} {branch}");
            }
            compared += 1;
        }
    }
    compared
}

fn py_repr(s: &str) -> String {
    // Fixture branches are plain ASCII without quotes.
    format!("'{s}'")
}

#[test]
fn json_claim_matches_the_reference_goldens() {
    assert!(check_against_golden("claim.json", &[]) >= 15);
}

#[test]
fn json_strict_matches_the_reference_goldens() {
    assert!(check_against_golden("strict.json", &["--strict"]) >= 15);
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
    let slug = common::slug_for(&common::canon(&world.repo)).to_lowercase();
    assert!(err.starts_with("No transcript directory for "), "{err}");
    assert!(
        err.ends_with(&format!(" (expected slug {slug})\n")),
        "{err}"
    );
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
