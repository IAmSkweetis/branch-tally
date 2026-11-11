//! End-to-end tests for redaction (#9): `--redact`, `--no-redact`, `--keep-names`,
//! `--redact-names`, the `redact`/`redact-names` config keys, and `config show`.
//!
//! `--pr` talks to the real `gh`, which is not faked in this crate; the redacted PR body is the
//! same Markdown these tests snapshot, and the splice is covered by core's fake `Gh`.

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, canon, slug_for, stderr, stdout};
use serde_json::Value;
use std::fs;

const BRANCH: &str = "feat/x";
const FOOTNOTE: &str = "identifiers redacted";
const DATE_HEADER: &str = "Span (UTC date)";

fn run_ok(world: &World, args: &[&str]) -> String {
    let out = world.run(args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    stdout(&out)
}

#[test]
fn redact_markdown_discloses_and_reduces_timestamps_to_dates() {
    let world = World::scenario("subagent");
    let text = run_ok(&world, &["report", BRANCH, "--redact"]);
    assert!(text.contains(FOOTNOTE), "{text}");
    assert!(text.contains(DATE_HEADER), "{text}");
    insta::assert_snapshot!("markdown_redacted", text);
}

#[test]
fn redact_with_no_cost_snapshot() {
    let world = World::scenario("subagent");
    let text = run_ok(&world, &["report", BRANCH, "--redact", "--no-cost"]);
    assert!(!text.contains("Est. cost"), "{text}");
    insta::assert_snapshot!("markdown_redacted_no_cost", text);
}

#[test]
fn redact_with_keep_names_snapshot() {
    let world = World::scenario("subagent");
    let text = run_ok(&world, &["report", BRANCH, "--redact", "--keep-names"]);
    insta::assert_snapshot!("markdown_redacted_keep_names", text);
}

#[test]
fn redact_json_snapshot() {
    let world = World::scenario("subagent");
    let text = run_ok(&world, &["report", BRANCH, "--redact", "--json"]);
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["redacted"], Value::Bool(true));
    insta::assert_snapshot!("json_redacted", text);
}

#[test]
fn cost_and_redact_are_independent_switches() {
    let world = World::scenario("subagent");
    for (extra, cost, redacted) in [
        (&[][..], true, false),
        (&["--no-cost"][..], false, false),
        (&["--redact"][..], true, true),
        (&["--redact", "--no-cost"][..], false, true),
    ] {
        let mut args = vec!["report", BRANCH];
        args.extend_from_slice(extra);
        let text = run_ok(&world, &args);
        assert_eq!(text.contains("Est. cost"), cost, "{extra:?}");
        assert_eq!(text.contains(FOOTNOTE), redacted, "{extra:?}");
        assert_eq!(text.contains(DATE_HEADER), redacted, "{extra:?}");
    }
}

#[test]
fn unredacted_output_is_unchanged_and_has_no_redacted_key() {
    let world = World::scenario("subagent");
    let text = run_ok(&world, &["report", BRANCH]);
    assert!(
        !text.contains(FOOTNOTE) && !text.contains(DATE_HEADER),
        "{text}"
    );
    let json: Value = serde_json::from_str(&run_ok(&world, &["report", BRANCH, "--json"])).unwrap();
    assert!(json.get("redacted").is_none());
}

#[test]
fn redact_flags_conflict_in_pairs() {
    let world = World::scenario("subagent");
    for pair in [
        ["--redact", "--no-redact"],
        ["--keep-names", "--redact-names"],
    ] {
        let out = world.run(&["report", BRANCH, pair[0], pair[1]]);
        assert_eq!(out.status.code(), Some(2), "{pair:?}");
        assert_eq!(stdout(&out), "");
    }
}

#[test]
fn config_redact_applies_and_no_redact_undoes_it() {
    let world = World::scenario("subagent");
    world.write_default_config("redact = true\n");
    assert!(run_ok(&world, &["report", BRANCH]).contains(FOOTNOTE));
    assert!(!run_ok(&world, &["report", BRANCH, "--no-redact"]).contains(FOOTNOTE));
    world.write_default_config("redact = false\n");
    assert!(run_ok(&world, &["report", BRANCH, "--redact"]).contains(FOOTNOTE));
}

#[test]
fn keep_names_without_redaction_is_harmless() {
    let world = World::scenario("subagent");
    let plain = run_ok(&world, &["report", BRANCH]);
    assert_eq!(run_ok(&world, &["report", BRANCH, "--keep-names"]), plain);
}

#[test]
fn config_redact_names_false_keeps_names_and_flag_overrides_it() {
    let world = World::scenario("subagent");
    let kept = run_ok(&world, &["report", BRANCH, "--redact", "--keep-names"]);
    world.write_default_config("redact = true\nredact-names = false\n");
    assert_eq!(run_ok(&world, &["report", BRANCH]), kept);
    let generic = run_ok(&world, &["report", BRANCH, "--redact-names"]);
    assert_ne!(generic, kept);
}

#[test]
fn config_show_reports_redact_settings_and_sources() {
    let world = World::empty();
    let defaults = run_ok(&world, &["config", "show"]);
    assert!(defaults.contains("redact = false (default)"), "{defaults}");
    assert!(
        defaults.contains("redact-names = true (default)"),
        "{defaults}"
    );
    world.write_default_config("redact = true\nredact-names = false\n");
    let set = run_ok(&world, &["config", "show"]);
    assert!(set.contains("redact = true (config file)"), "{set}");
    assert!(set.contains("redact-names = false (config file)"), "{set}");
}

/// A synthetic transcript whose ids and times are unmistakable, so any survivor is a leak.
#[test]
fn redacted_output_carries_no_session_id_agent_id_or_time_of_day() {
    const SESSION: &str = "sess-9f3c-zebra";
    const AGENT: &str = "agentqx7";
    const TIME: &str = "13:47:22";
    let world = World::empty();
    let dir = world.projects().join(slug_for(&canon(&world.repo)));
    fs::create_dir_all(&dir).unwrap();
    let cwd = canon(&world.repo)
        .display()
        .to_string()
        .replace('\\', "\\\\");
    let line = |uuid: &str, branch: &str, extra: &str| {
        format!(
            "{{\"type\":\"assistant\",\"requestId\":\"req_{uuid}\",\"uuid\":\"{uuid}\",\
             \"sessionId\":\"{SESSION}\",\"timestamp\":\"2026-03-04T{TIME}.000Z\",\
             \"cwd\":\"{cwd}\",\"gitBranch\":\"{branch}\",{extra}\
             \"message\":{{\"model\":\"claude-test-1\",\"content\":\"PLACEHOLDER\",\
             \"usage\":{{\"input_tokens\":10,\"output_tokens\":20}}}}}}\n"
        )
    };
    // The dispatcher's own call, then the link naming the subagent it launched.
    let own_call = line("m1", "feat/leak", "\"isSidechain\":false,");
    let dispatch = format!(
        "{{\"type\":\"user\",\"uuid\":\"u_disp\",\"sessionId\":\"{SESSION}\",\
         \"timestamp\":\"2026-03-04T{TIME}.000Z\",\"gitBranch\":\"feat/leak\",\"cwd\":\"{cwd}\",\
         \"isSidechain\":false,\"sourceToolAssistantUUID\":\"u_tool\",\
         \"toolUseResult\":{{\"agentId\":\"{AGENT}\",\"isAsync\":true}}}}\n"
    );
    fs::write(
        dir.join(format!("{SESSION}.jsonl")),
        format!("{own_call}{dispatch}"),
    )
    .unwrap();
    let sub = dir.join(SESSION).join("subagents");
    fs::create_dir_all(&sub).unwrap();
    let sub_line = line(
        "m2",
        "worktree-agent-qx7",
        &format!("\"isSidechain\":true,\"agentId\":\"{AGENT}\","),
    );
    fs::write(sub.join(format!("agent-{AGENT}.jsonl")), sub_line).unwrap();

    // Control: unredacted output does show them, so the assertions below can fail.
    let plain = run_ok(&world, &["report", "feat/leak"]);
    // Markdown shows only the first eight characters of a session id.
    assert!(plain.contains(&SESSION[..8]), "{plain}");
    assert!(plain.contains("13:47"), "{plain}");
    assert!(plain.contains(AGENT), "{plain}");

    for extra in [&["--redact"][..], &["--redact", "--json"][..]] {
        let mut args = vec!["report", "feat/leak"];
        args.extend_from_slice(extra);
        let text = run_ok(&world, &args);
        for secret in [SESSION, &SESSION[..8], AGENT, TIME, "13:47"] {
            assert!(!text.contains(secret), "{extra:?} leaked {secret}: {text}");
        }
        assert!(text.contains("2026-03-04"), "{extra:?}: dates stay");
    }
}
