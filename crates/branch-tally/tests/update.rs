//! End-to-end: `branch-tally update` (alias `upgrade`) as the real binary shows it.
//!
//! The test binary is a development build (no `BRANCH_TALLY_VERSION`), so what it can show is the
//! help text, the alias and the dev-build refusal. Replacing a binary is covered by unit tests of
//! the real client and installer in `src/update.rs`, on temp directories.

#![expect(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_branch-tally"))
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn help_lists_the_four_flags() {
    let out = run(&["update", "--help"]);
    assert!(out.status.success());
    let help = text(&out.stdout);
    for flag in ["--check", "--version", "--yes", "--force"] {
        assert!(help.contains(flag), "missing {flag} in:\n{help}");
    }
}

#[test]
fn a_dev_build_refuses_with_exit_1_and_a_message_on_stderr() {
    let out = run(&["update", "--check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("development build"));
}

#[test]
fn upgrade_is_an_alias_for_update() {
    let out = run(&["upgrade", "--yes"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("development build"));
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    assert_eq!(run(&["update", "--bogus"]).status.code(), Some(2));
}

fn run_with_base(base: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_branch-tally"))
        .args(["update", "--check"])
        .env("BRANCH_TALLY_INSTALL_BASE_URL", base)
        .output()
        .unwrap()
}

#[test]
fn a_plain_http_mirror_is_refused_before_any_network_call() {
    let out = run_with_base("http://127.0.0.1:9/releases");
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(err.contains("BRANCH_TALLY_INSTALL_BASE_URL"), "{err}");
    assert!(err.contains("https://"), "{err}");
    assert_eq!(err.trim().lines().count(), 1, "{err}");
}

#[test]
fn a_file_mirror_is_accepted_and_reaches_the_dev_build_refusal() {
    let out = run_with_base("file:///nonexistent");
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("development build"));
}

#[test]
fn long_help_says_what_is_verified_and_how_to_verify_an_old_release() {
    let out = run(&["update", "--help"]);
    let help = text(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for needle in [
        "SHA256SUMS.minisig",
        "maintainer's key",
        "integrity check",
        "before signing are refused",
        "gh attestation verify",
        "SECURITY.md",
    ] {
        assert!(help.contains(needle), "missing {needle:?} in:\n{help}");
    }
}

#[test]
fn there_is_no_flag_to_replace_the_release_key() {
    for flag in ["--key", "--public-key", "--release-key"] {
        assert_eq!(run(&["update", flag, "x"]).status.code(), Some(2), "{flag}");
    }
}
