//! Integration tests for `SystemGit` against a real temporary repository.
//!
//! These REQUIRE `git` on `PATH`. They fail loudly (never skip) when it is missing.

#![expect(
    clippy::expect_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use branch_tally_core::git::{Git, GitError, SystemGit, normalize_path};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("these tests require `git` on PATH");
    assert!(out.status.success(), "git {args:?} failed");
}

/// A repo on branch `trunk` with one commit, plus a worktree `wt` on branch `feat/wt`.
struct Fixture {
    _tmp: tempfile::TempDir,
    main: PathBuf,
    wt: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let main = tmp.path().join("main repo");
    std::fs::create_dir(&main).expect("mkdir");
    git(&main, &["init", "-q", "-b", "trunk"]);
    git(&main, &["config", "user.email", "t@example.invalid"]);
    git(&main, &["config", "user.name", "t"]);
    git(&main, &["config", "commit.gpgsign", "false"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = tmp.path().join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/wt",
            wt.to_str().expect("utf8"),
        ],
    );
    let main = normalize_path(&main);
    let wt = normalize_path(&wt);
    Fixture {
        _tmp: tmp,
        main,
        wt,
    }
}

#[test]
fn current_branch_in_main_and_worktree() {
    let f = fixture();
    assert_eq!(SystemGit.current_branch(&f.main).expect("main"), "trunk");
    assert_eq!(SystemGit.current_branch(&f.wt).expect("wt"), "feat/wt");
}

#[test]
fn main_checkout_from_worktree_is_the_main_repo() {
    let f = fixture();
    assert_eq!(SystemGit.main_checkout(&f.wt), f.main);
    assert_eq!(SystemGit.main_checkout(&f.main), f.main);
}

#[test]
fn main_checkout_falls_back_outside_a_repo() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert_eq!(SystemGit.main_checkout(tmp.path()), tmp.path());
}

#[test]
fn worktrees_lists_both_with_clean_paths() {
    let f = fixture();
    let got = SystemGit.worktrees(&f.main).expect("worktrees");
    let find = |b: &str| got.iter().find(|w| w.branch == b).map(|w| w.path.clone());
    assert_eq!(find("trunk"), Some(f.main.clone()));
    assert_eq!(find("feat/wt"), Some(f.wt.clone()));
    assert!(
        got.iter()
            .all(|w| !w.path.to_string_lossy().starts_with(r"\\?\")),
        "verbatim prefix leaked"
    );
}

#[test]
fn detached_worktree_is_omitted() {
    let f = fixture();
    git(&f.wt, &["checkout", "-q", "--detach"]);
    let got = SystemGit.worktrees(&f.main).expect("worktrees");
    assert!(got.iter().all(|w| w.branch != "feat/wt"));
    assert_eq!(got.len(), 1);
}

#[test]
fn failure_outside_a_repo_is_observable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(matches!(
        SystemGit.worktrees(tmp.path()),
        Err(GitError::Failed { .. })
    ));
    assert!(matches!(
        SystemGit.current_branch(tmp.path()),
        Err(GitError::Failed { .. })
    ));
}
