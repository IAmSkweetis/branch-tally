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
fn toplevel_from_a_subdirectory_of_the_main_checkout_is_its_root() {
    let f = fixture();
    let sub = f.main.join("a").join("b");
    std::fs::create_dir_all(&sub).expect("mkdir");
    assert_eq!(SystemGit.toplevel(&sub).expect("toplevel"), f.main);
    assert_eq!(SystemGit.toplevel(&f.main).expect("root"), f.main);
}

#[test]
fn toplevel_from_a_subdirectory_of_a_linked_worktree_is_the_worktree_root() {
    let f = fixture();
    let sub = f.wt.join("deep").join("er");
    std::fs::create_dir_all(&sub).expect("mkdir");
    let got = SystemGit.toplevel(&sub).expect("toplevel");
    assert_eq!(got, f.wt);
    assert_ne!(got, f.main);
}

#[test]
fn toplevel_outside_a_repo_is_a_typed_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let err = SystemGit.toplevel(tmp.path()).expect_err("not a repo");
    assert!(matches!(err, GitError::Failed { .. }), "got {err:?}");
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
fn default_branch_is_not_guessed_without_origin_head() {
    // `trunk` has no remote at all: no `main` guess, and no local-branch guess either.
    let f = fixture();
    assert!(matches!(
        SystemGit.default_branch(&f.main),
        Err(GitError::Failed { .. })
    ));
}

#[test]
fn default_branch_reads_origin_head_and_strips_the_remote() {
    let f = fixture();
    let origin = f.main.to_str().expect("utf8");
    git(&f.wt, &["remote", "add", "origin", origin]);
    git(&f.wt, &["fetch", "-q", "origin"]);
    git(&f.wt, &["remote", "set-head", "origin", "trunk"]);
    assert_eq!(SystemGit.default_branch(&f.wt).expect("default"), "trunk");
    // Shared config: the main checkout sees the same remote.
    assert_eq!(SystemGit.default_branch(&f.main).expect("default"), "trunk");
}

#[test]
fn default_branch_keeps_slashes_inside_the_branch_name() {
    let f = fixture();
    let origin = f.main.to_str().expect("utf8");
    git(&f.main, &["remote", "add", "origin", origin]);
    git(&f.main, &["fetch", "-q", "origin"]);
    git(&f.main, &["remote", "set-head", "origin", "feat/wt"]);
    assert_eq!(
        SystemGit.default_branch(&f.main).expect("default"),
        "feat/wt"
    );
}

#[test]
fn default_branch_outside_a_repo_is_an_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(SystemGit.default_branch(tmp.path()).is_err());
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

fn commit_file(dir: &Path, rel: &str, bytes: &[u8]) {
    let full = dir.join(rel);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    std::fs::write(&full, bytes).expect("write");
    git(dir, &["add", rel]);
    git(dir, &["commit", "-q", "-m", "add"]);
}

#[test]
fn file_at_returns_the_bytes_at_the_ref_not_the_working_tree() {
    let f = fixture();
    let bytes = b"one\r\n\xff\x00two\n";
    commit_file(&f.main, ".tally/ledger.jsonl", bytes);
    git(&f.main, &["branch", "base"]);
    std::fs::write(f.main.join(".tally/ledger.jsonl"), b"edited").expect("write");
    let got = SystemGit
        .file_at(&f.main, "base", Path::new(".tally/ledger.jsonl"))
        .expect("file_at");
    assert_eq!(got.as_deref(), Some(&bytes[..]));
}

#[test]
fn file_at_builds_the_spec_from_components_not_os_separators() {
    let f = fixture();
    commit_file(&f.main, "a/b/c.txt", b"x");
    let nested: PathBuf = ["a", "b", "c.txt"].iter().collect();
    let got = SystemGit.file_at(&f.main, "HEAD", &nested).expect("ok");
    assert_eq!(got.as_deref(), Some(&b"x"[..]));
}

#[test]
fn file_at_missing_path_is_none() {
    let f = fixture();
    let got = SystemGit
        .file_at(&f.main, "HEAD", Path::new(".tally/ledger.jsonl"))
        .expect("ok");
    assert_eq!(got, None);
}

#[test]
fn file_at_unknown_ref_is_a_typed_error_not_none() {
    let f = fixture();
    let err = SystemGit
        .file_at(&f.main, "no-such-ref", Path::new("x"))
        .expect_err("unknown ref");
    assert!(matches!(err, GitError::Failed { .. }), "got {err:?}");
}

#[test]
fn file_at_refuses_a_rev_that_looks_like_an_option() {
    let f = fixture();
    let err = SystemGit
        .file_at(&f.main, "--help", Path::new("x"))
        .expect_err("option injection");
    assert!(
        matches!(err, GitError::InvalidArgument { what: "rev" }),
        "got {err:?}"
    );
}

#[test]
fn file_at_refuses_paths_that_are_not_repository_relative() {
    let f = fixture();
    for bad in ["", "../x", "a/../../x", "/etc/passwd"] {
        let err = SystemGit
            .file_at(&f.main, "HEAD", Path::new(bad))
            .expect_err(bad);
        assert!(
            matches!(err, GitError::InvalidArgument { what: "path" }),
            "{bad:?}: {err:?}"
        );
    }
}

#[test]
fn file_at_outside_a_repo_is_a_typed_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let err = SystemGit
        .file_at(tmp.path(), "HEAD", Path::new("x"))
        .expect_err("not a repo");
    assert!(matches!(err, GitError::Failed { .. }), "got {err:?}");
}

#[test]
fn file_at_error_text_carries_no_file_contents() {
    let f = fixture();
    commit_file(&f.main, "secret.txt", b"TOPSECRET-CONTENT");
    let err = SystemGit
        .file_at(&f.main, "nope", Path::new("secret.txt"))
        .expect_err("unknown ref");
    assert!(!format!("{err} {err:?}").contains("TOPSECRET"));
}
