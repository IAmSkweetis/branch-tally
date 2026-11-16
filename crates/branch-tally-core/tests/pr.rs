//! Tests for `update_pr` against a scripted `gh`. No network, no real GitHub.

#![expect(clippy::panic, reason = "a wrong error variant fails the test loudly")]

use branch_tally_core::pr::{FakeGh, GhError, PrError, PrOutcome, update_pr};
use branch_tally_core::render::{END, START, wrap};

fn list() -> Vec<&'static str> {
    vec![
        "pr",
        "list",
        "--repo",
        REPO,
        "--head",
        "feat/x",
        "--state",
        "open",
        "--json",
        "number,headRepositoryOwner,isCrossRepository",
        "--limit",
        "100",
    ]
}

fn view(n: &str) -> Vec<&str> {
    vec!["pr", "view", n, "--repo", REPO, "--json", "body"]
}

fn edit(n: &str) -> Vec<&str> {
    vec!["pr", "edit", n, "--repo", REPO, "--body-file", "-"]
}

const REPO: &str = "Acme/widgets";

const REPORT: &str = "| a | b |\n|---|---|\n| \u{2192} | \u{b7} |";

#[test]
fn finds_pr_then_writes_spliced_body_on_stdin() {
    let gh = FakeGh::new()
        .on(&list(), Ok(r#"[{"number": 7, "isCrossRepository": false, "headRepositoryOwner": {"login": "acme"}}]"#))
        .on(&view("7"), Ok(r#"{"body": "intro"}"#))
        .on(&edit("7"), Ok(""));
    let out = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap();
    assert_eq!(out, PrOutcome::Written { number: 7 });
    let calls = gh.calls();
    assert_eq!(calls.len(), 3);
    let stdin = calls[2].stdin.as_deref().unwrap();
    assert_eq!(
        stdin,
        format!("intro\n\n## Token usage\n\n{}\n", wrap(REPORT))
    );
    assert!(!stdin.contains('\r'));
}

#[test]
fn explicit_number_skips_the_search() {
    let gh = FakeGh::new()
        .on(&view("12"), Ok(r#"{"body": null}"#))
        .on(&edit("12"), Ok(""));
    let out = update_pr(&gh, "feat/x", REPORT, REPO, Some(12), false).unwrap();
    assert_eq!(out, PrOutcome::Written { number: 12 });
    assert_eq!(gh.calls().len(), 2);
}

#[test]
fn replaces_the_marked_region_in_place_never_stacking() {
    let body = format!("top\\n{START}\\nold\\n{END}\\nbottom");
    let gh = FakeGh::new()
        .on(&view("3"), Ok(&format!("{{\"body\": \"{body}\"}}")))
        .on(&edit("3"), Ok(""));
    update_pr(&gh, "feat/x", REPORT, REPO, Some(3), false).unwrap();
    let written = gh.calls()[1].stdin.clone().unwrap();
    assert_eq!(written, format!("top\n{}\nbottom", wrap(REPORT)));
    assert_eq!(written.matches(START).count(), 1);
}

#[test]
fn up_to_date_ignores_crlf_and_surrounding_whitespace() {
    let body = format!("top\r\n{}\r\n\r\n", wrap(REPORT))
        .replace("\r\n", "\\r\\n")
        .replace('\n', "\\n");
    let gh = FakeGh::new().on(&view("3"), Ok(&format!("{{\"body\": \"{body}\"}}")));
    let out = update_pr(&gh, "feat/x", REPORT, REPO, Some(3), false).unwrap();
    assert_eq!(out, PrOutcome::UpToDate { number: 3 });
    assert_eq!(gh.calls().len(), 1);
}

#[test]
fn dry_run_returns_the_body_and_never_edits() {
    let gh = FakeGh::new().on(&view("3"), Ok(r#"{"body": "x"}"#));
    let out = update_pr(&gh, "feat/x", REPORT, REPO, Some(3), true).unwrap();
    assert_eq!(
        out,
        PrOutcome::DryRun {
            number: 3,
            body: format!("x\n\n## Token usage\n\n{}\n", wrap(REPORT)),
        }
    );
    assert!(gh.calls().iter().all(|c| c.args[1] != "edit"));
}

#[test]
fn no_open_pr_is_an_error_for_empty_list_and_empty_output() {
    for stdout in ["[]", ""] {
        let gh = FakeGh::new().on(&list(), Ok(stdout));
        let err = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap_err();
        assert!(matches!(err, PrError::NoOpenPr { ref branch, ignored: 0 } if branch == "feat/x"));
    }
}

#[test]
fn gh_failure_carries_args_and_detail() {
    let gh = FakeGh::new().on(&list(), Err("boom"));
    let err = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap_err();
    let PrError::Gh(GhError::Failed { args, detail }) = err else {
        panic!("wrong variant");
    };
    assert_eq!(detail, "boom");
    assert!(args.starts_with("pr list --repo Acme/widgets --head feat/x"));
}

#[test]
fn unparseable_output_is_a_typed_error_not_a_panic() {
    let gh = FakeGh::new().on(&view("3"), Ok("not json"));
    let err = update_pr(&gh, "feat/x", REPORT, REPO, Some(3), false).unwrap_err();
    assert!(matches!(err, PrError::Gh(GhError::BadOutput { .. })));
}

fn pr_json(items: &[(u64, bool, &str)]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|(n, cross, owner)| {
            format!(
                r#"{{"number": {n}, "isCrossRepository": {cross}, "headRepositoryOwner": {{"login": "{owner}"}}}}"#
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

#[test]
fn fork_pr_is_ignored_and_counted_in_no_open_pr() {
    let gh = FakeGh::new().on(&list(), Ok(&pr_json(&[(9, true, "mallory")])));
    let err = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap_err();
    assert!(
        matches!(err, PrError::NoOpenPr { ref branch, ignored: 1 } if branch == "feat/x"),
        "{err:?}"
    );
}

#[test]
fn same_repo_pr_wins_over_a_fork_pr_and_owner_matches_case_insensitively() {
    let gh = FakeGh::new()
        .on(
            &list(),
            Ok(&pr_json(&[(9, true, "mallory"), (4, false, "ACME")])),
        )
        .on(&view("4"), Ok(r#"{"body": "x"}"#))
        .on(&edit("4"), Ok(""));
    let out = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap();
    assert_eq!(out, PrOutcome::Written { number: 4 });
}

#[test]
fn non_cross_pr_with_a_different_owner_is_not_kept() {
    let gh = FakeGh::new().on(&list(), Ok(&pr_json(&[(4, false, "other")])));
    let err = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap_err();
    assert!(
        matches!(err, PrError::NoOpenPr { ignored: 1, .. }),
        "{err:?}"
    );
}

#[test]
fn several_matches_fail_listing_sorted_numbers_and_never_edit() {
    let gh = FakeGh::new().on(
        &list(),
        Ok(&pr_json(&[(12, false, "acme"), (3, false, "acme")])),
    );
    let err = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap_err();
    assert!(
        matches!(err, PrError::AmbiguousPr { ref branch, ref numbers } if branch == "feat/x" && numbers == &[3, 12]),
        "{err:?}"
    );
    assert_eq!(gh.calls().len(), 1);
}

#[test]
fn missing_fields_are_bad_output_never_a_guess() {
    for json in [
        r#"[{"number": 7, "headRepositoryOwner": {"login": "acme"}}]"#,
        r#"[{"number": 7, "isCrossRepository": false}]"#,
        r#"[{"isCrossRepository": false, "headRepositoryOwner": {"login": "acme"}}]"#,
        r#"{"number": 7}"#,
    ] {
        let gh = FakeGh::new().on(&list(), Ok(json));
        let err = update_pr(&gh, "feat/x", REPORT, REPO, None, false).unwrap_err();
        assert!(
            matches!(err, PrError::Gh(GhError::BadOutput { .. })),
            "{json}: {err:?}"
        );
    }
}
