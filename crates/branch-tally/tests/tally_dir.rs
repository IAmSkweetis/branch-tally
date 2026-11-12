//! End-to-end tests for the `.tally/` symlink guard and opt-in rule (#94): the binary never
//! follows a link out of `.tally/` and never creates the directory. An explicit `--ledger` keeps
//! its plain behaviour.

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
use std::fs;
use std::io::Write;
use std::process::{Output, Stdio};

const BRANCH: &str = "feat/new";
const VICTIM_TEXT: &str = "victim-original-bytes\n";

fn capture(world: &World) -> Output {
    let payload = format!(
        r#"{{"hook_event_name":"Stop","session_id":"s1","cwd":{}}}"#,
        serde_json::to_string(&world.repo.to_string_lossy()).unwrap()
    );
    let mut child = world
        .command(&world.repo, &["capture"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
fn victim(world: &World) -> std::path::PathBuf {
    let path = world.home.join("victim.txt");
    fs::write(&path, VICTIM_TEXT).unwrap();
    path
}

#[cfg(unix)]
fn link(target: &std::path::Path, at: &std::path::Path) {
    std::os::unix::fs::symlink(target, at).unwrap();
}

#[test]
fn capture_without_tally_dir_writes_nothing_and_is_silent() {
    let world = World::empty();
    let out = capture(&world);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(stderr(&out), "");
    assert!(!world.repo.join(".tally").exists());
}

#[test]
fn record_without_tally_dir_writes_nothing_and_says_so() {
    let world = World::scenario("main-then-branch");
    let out = world.run(&["record", BRANCH]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!world.repo.join(".tally").exists());
    assert_eq!(
        stdout(&out),
        "no .tally/ directory: nothing recorded (create it to opt in)\n"
    );
}

#[cfg(unix)]
#[test]
fn capture_refuses_a_symlinked_captures_file_and_leaves_the_victim_alone() {
    let world = World::empty();
    let victim = victim(&world);
    fs::create_dir(world.repo.join(".tally")).unwrap();
    link(&victim, &world.repo.join(".tally").join("captures.jsonl"));
    let out = capture(&world);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM_TEXT);
    let err = stderr(&out);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(!err.contains("bt-e2e-"), "no cwd echoed: {err}");
}

#[cfg(unix)]
#[test]
fn capture_refuses_a_symlinked_tally_dir() {
    let world = World::empty();
    let elsewhere = world.home.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    link(&elsewhere, &world.repo.join(".tally"));
    let out = capture(&world);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn record_refuses_a_symlinked_ledger_and_leaves_the_victim_alone() {
    let world = World::scenario("main-then-branch");
    let victim = victim(&world);
    fs::create_dir(world.repo.join(".tally")).unwrap();
    link(&victim, &world.repo.join(".tally").join("ledger.jsonl"));
    let out = world.run(&["record", BRANCH]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM_TEXT);
    assert!(stderr(&out).contains("ledger.jsonl"), "{}", stderr(&out));
}

#[cfg(unix)]
#[test]
fn record_refuses_a_symlinked_tally_dir() {
    let world = World::scenario("main-then-branch");
    let elsewhere = world.home.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    link(&elsewhere, &world.repo.join(".tally"));
    let out = world.run(&["record", BRANCH]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn report_and_project_refuse_a_symlinked_default_ledger() {
    let world = World::scenario("main-then-branch");
    let victim = victim(&world);
    fs::create_dir(world.repo.join(".tally")).unwrap();
    link(&victim, &world.repo.join(".tally").join("ledger.jsonl"));
    for args in [&["report", BRANCH, "--from-ledger"][..], &["project"][..]] {
        let out = world.run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stdout(&out));
        assert!(stderr(&out).contains("ledger.jsonl"), "{}", stderr(&out));
    }
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM_TEXT);
}

#[cfg(unix)]
#[test]
fn a_symlinked_prices_toml_is_refused() {
    let world = World::scenario("main-then-branch");
    let target = world.home.join("prices-elsewhere.toml");
    fs::write(&target, "as_of = \"2030-01-02\"\n").unwrap();
    fs::create_dir(world.repo.join(".tally")).unwrap();
    link(&target, &world.repo.join(".tally").join("prices.toml"));
    let out = world.run(&["report", BRANCH]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let err = stderr(&out);
    assert!(err.contains("prices.toml"), "{err}");
    // Core words a linked file as "not a regular file"; only a linked directory says "symbolic link".
    assert!(err.contains("refusing to use"), "{err}");
    assert!(err.contains("not a regular file"), "{err}");
    assert!(!err.contains("other error"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_tally_dir_is_refused_with_its_reason_by_report_and_project() {
    let world = World::scenario("main-then-branch");
    let elsewhere = world.home.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    link(&elsewhere, &world.repo.join(".tally"));
    for args in [&["report", BRANCH][..], &["project"][..]] {
        let out = world.run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stdout(&out));
        let err = stderr(&out);
        assert!(err.contains("symbolic link"), "{args:?}: {err}");
        assert!(!err.contains("other error"), "{args:?}: {err}");
    }
}

#[cfg(unix)]
#[test]
fn an_explicit_ledger_symlink_still_works() {
    let world = World::scenario("main-then-branch");
    let real = world.home.join("real-ledger.jsonl");
    fs::write(&real, "").unwrap();
    let alias = world.home.join("alias-ledger.jsonl");
    link(&real, &alias);
    let path = alias.to_str().unwrap();
    let rec = world.run(&["record", BRANCH, "--ledger", path]);
    assert_eq!(rec.status.code(), Some(0), "{}", stderr(&rec));
    assert_eq!(fs::read_to_string(&real).unwrap().lines().count(), 5);
    let rep = world.run(&["report", BRANCH, "--from-ledger", "--ledger", path]);
    assert_eq!(rep.status.code(), Some(0), "{}", stderr(&rep));
}
