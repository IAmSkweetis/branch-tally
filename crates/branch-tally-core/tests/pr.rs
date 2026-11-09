//! Tests for `update_pr` against a scripted `gh`. No network, no real GitHub.

#![expect(clippy::panic, reason = "a wrong error variant fails the test loudly")]

use branch_tally_core::pr::{FakeGh, GhError, PrError, PrOutcome, update_pr};
use branch_tally_core::render::{END, START, wrap};

fn list() -> Vec<&'static str> {
    vec![
        "pr", "list", "--head", "feat/x", "--state", "open", "--json", "number", "--limit", "1",
    ]
}

fn view(n: &str) -> Vec<&str> {
    vec!["pr", "view", n, "--json", "body"]
}

fn edit(n: &str) -> Vec<&str> {
    vec!["pr", "edit", n, "--body-file", "-"]
}

const REPORT: &str = "| a | b |\n|---|---|\n| \u{2192} | \u{b7} |";

#[test]
fn finds_pr_then_writes_spliced_body_on_stdin() {
    let gh = FakeGh::new()
        .on(&list(), Ok(r#"[{"number": 7}]"#))
        .on(&view("7"), Ok(r#"{"body": "intro"}"#))
        .on(&edit("7"), Ok(""));
    let out = update_pr(&gh, "feat/x", REPORT, None, false).unwrap();
    assert_eq!(out, PrOutcome::Written { number: "7".into() });
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
    let out = update_pr(&gh, "feat/x", REPORT, Some("12"), false).unwrap();
    assert_eq!(
        out,
        PrOutcome::Written {
            number: "12".into()
        }
    );
    assert_eq!(gh.calls().len(), 2);
}

#[test]
fn replaces_the_marked_region_in_place_never_stacking() {
    let body = format!("top\\n{START}\\nold\\n{END}\\nbottom");
    let gh = FakeGh::new()
        .on(&view("3"), Ok(&format!("{{\"body\": \"{body}\"}}")))
        .on(&edit("3"), Ok(""));
    update_pr(&gh, "feat/x", REPORT, Some("3"), false).unwrap();
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
    let out = update_pr(&gh, "feat/x", REPORT, Some("3"), false).unwrap();
    assert_eq!(out, PrOutcome::UpToDate { number: "3".into() });
    assert_eq!(gh.calls().len(), 1);
}

#[test]
fn dry_run_returns_the_body_and_never_edits() {
    let gh = FakeGh::new().on(&view("3"), Ok(r#"{"body": "x"}"#));
    let out = update_pr(&gh, "feat/x", REPORT, Some("3"), true).unwrap();
    assert_eq!(
        out,
        PrOutcome::DryRun {
            number: "3".into(),
            body: format!("x\n\n## Token usage\n\n{}\n", wrap(REPORT)),
        }
    );
    assert!(gh.calls().iter().all(|c| c.args[1] != "edit"));
}

#[test]
fn no_open_pr_is_an_error_for_empty_list_and_empty_output() {
    for stdout in ["[]", ""] {
        let gh = FakeGh::new().on(&list(), Ok(stdout));
        let err = update_pr(&gh, "feat/x", REPORT, None, false).unwrap_err();
        assert!(matches!(err, PrError::NoOpenPr { ref branch } if branch == "feat/x"));
    }
}

#[test]
fn gh_failure_carries_args_and_detail() {
    let gh = FakeGh::new().on(&list(), Err("boom"));
    let err = update_pr(&gh, "feat/x", REPORT, None, false).unwrap_err();
    let PrError::Gh(GhError::Failed { args, detail }) = err else {
        panic!("wrong variant");
    };
    assert_eq!(detail, "boom");
    assert!(args.starts_with("pr list --head feat/x"));
}

#[test]
fn unparseable_output_is_a_typed_error_not_a_panic() {
    let gh = FakeGh::new().on(&view("3"), Ok("not json"));
    let err = update_pr(&gh, "feat/x", REPORT, Some("3"), false).unwrap_err();
    assert!(matches!(err, PrError::Gh(GhError::BadOutput { .. })));
}
