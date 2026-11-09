//! The Claude Code source, exercised against the hand-written fixtures in
//! `tests/fixtures/claude-code/`. Each scenario directory acts as the projects root.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers outside #[test] fns fail loudly on bad fixtures"
)]

use std::path::{Path, PathBuf};

use branch_tally_core::model::{AgentId, Harness, Record, SessionId};
use branch_tally_core::sources::claude_code::{ClaudeCode, slug_candidates};
use branch_tally_core::sources::{
    ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot,
};

fn fixture(scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("claude-code")
        .join(scenario)
}

fn names(roots: &[TranscriptRoot]) -> Vec<(String, RootKind)> {
    roots
        .iter()
        .map(|r| {
            (
                r.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_owned(),
                r.kind(),
            )
        })
        .collect()
}

fn discover(scenario: &str, repo: &str) -> Vec<TranscriptRoot> {
    ClaudeCode
        .discover(&fixture(scenario), &RepoIdentity::new(repo))
        .unwrap()
}

fn read_all(scenario: &str, repo: &str) -> (Vec<Record>, ReadStats) {
    let roots = discover(scenario, repo);
    let mut records = Vec::new();
    let stats = ClaudeCode.read(&roots, &mut |r| records.push(r)).unwrap();
    (records, stats)
}

fn by_request<'a>(records: &'a [Record], id: &str) -> &'a Record {
    records
        .iter()
        .find(|r| r.request_id.as_str() == id)
        .unwrap_or_else(|| panic!("no record {id}"))
}

#[test]
fn harness_is_claude_code() {
    assert_eq!(ClaudeCode.harness(), Harness::ClaudeCode);
}

#[test]
fn slug_replaces_every_non_alphanumeric_char() {
    assert_eq!(slug_candidates(Path::new(r"C:\repo")), ["C--repo"]);
    assert_eq!(slug_candidates(Path::new("/repo")), ["-repo"]);
    assert_eq!(
        slug_candidates(Path::new(r"D:\work spaces\my.repo_x")),
        ["D--work-spaces-my-repo-x"]
    );
}

#[test]
fn slug_accepts_git_bash_drive_form_and_verbatim_prefix() {
    let windows = slug_candidates(Path::new(r"C:\repo"));
    assert!(slug_candidates(Path::new("/c/repo")).contains(&windows[0]));
    // Case is not normalised here; matching against directory names is case-insensitive.
    assert_eq!(slug_candidates(Path::new("c:/repo")), ["c--repo"]);
    assert!(slug_candidates(Path::new(r"\\?\C:\repo")).contains(&windows[0]));
    // The literal reading stays a candidate, so a real Unix `/c/repo` still matches.
    assert!(slug_candidates(Path::new("/c/repo")).contains(&"-c-repo".to_owned()));
}

#[test]
fn discovery_includes_worktrees_and_excludes_prefix_siblings() {
    let roots = discover("worktree-and-siblings", r"C:\repo");
    let mut got = names(&roots);
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        got,
        vec![
            ("C--Repo--Claude-Worktrees-b".to_owned(), RootKind::Worktree),
            ("C--repo".to_owned(), RootKind::Repository),
            (
                "C--repo--claude-worktrees-feat-a".to_owned(),
                RootKind::Worktree
            ),
        ]
    );
}

#[test]
fn discovery_matches_slug_case_insensitively_for_every_path_form() {
    for repo in [r"C:\repo", "c:/repo", "/c/repo", "/C/repo"] {
        let roots = discover("slug-case", repo);
        assert_eq!(
            names(&roots),
            vec![("c--repo".to_owned(), RootKind::Repository)],
            "repo form {repo}"
        );
    }
}

#[test]
fn discovery_of_an_unrelated_repo_finds_nothing() {
    assert_eq!(
        discover("worktree-and-siblings", r"C:\nothing").len(),
        0,
        "no directory should match"
    );
}

#[test]
fn reading_worktree_roots_counts_files_and_records() {
    let (records, stats) = read_all("worktree-and-siblings", r"C:\repo");
    assert_eq!(records.len(), 3);
    assert_eq!(
        stats,
        ReadStats {
            files_read: 3,
            records_emitted: 3,
            ..ReadStats::default()
        }
    );
}

#[test]
fn replayed_history_is_deduped_across_roots() {
    let (records, stats) = read_all("replayed-history", r"C:\repo");
    assert_eq!(
        stats,
        ReadStats {
            files_read: 3,
            records_emitted: 3,
            duplicates_removed: 3,
            ..ReadStats::default()
        }
    );
    assert_eq!(records.len(), 3);
    assert_eq!(
        by_request(&records, "req_3").git_branch.as_deref(),
        Some("main")
    );
}

#[test]
fn synthetic_records_are_counted_and_not_emitted() {
    let (records, stats) = read_all("synthetic", r"C:\repo");
    assert_eq!(
        stats,
        ReadStats {
            files_read: 1,
            records_emitted: 1,
            synthetic: 1,
            ..ReadStats::default()
        }
    );
    assert_eq!(records[0].request_id.as_str(), "req_1");
}

#[test]
fn subagent_records_take_the_parent_session_from_the_directory() {
    let (records, stats) = read_all("subagent", r"C:\repo");
    assert_eq!(
        stats,
        ReadStats {
            files_read: 2,
            records_emitted: 3,
            ..ReadStats::default()
        }
    );
    let main = by_request(&records, "req_1");
    assert_eq!(main.parent_session, None);
    assert_eq!(main.agent_id, None);
    assert!(!main.is_sidechain);
    assert_eq!(main.git_branch.as_deref(), Some("feat/x"));

    let sub = by_request(&records, "req_a2");
    assert_eq!(
        sub.parent_session.as_ref().map(SessionId::as_str),
        Some("s1")
    );
    assert_eq!(sub.session_id.as_str(), "s1");
    assert_eq!(sub.agent_id.as_ref().map(AgentId::as_str), Some("a1"));
    assert!(sub.is_sidechain);
    // Raw stamp, never rewritten.
    assert_eq!(sub.git_branch.as_deref(), Some("worktree-agent-a1"));
    assert_eq!(sub.cwd.as_deref(), Some(r"C:\repo"));
    assert_eq!(sub.ts.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    assert_eq!(sub.model.as_deref(), Some("claude-test-1"));
}

#[test]
fn malformed_keyless_and_non_usage_lines_are_counted_correctly() {
    let (records, stats) = read_all("malformed-and-keyless", r"C:\repo");
    assert_eq!(
        stats,
        ReadStats {
            files_read: 1,
            records_emitted: 2,
            malformed_lines: 2,
            no_dedupe_key: 1,
            synthetic: 0,
            duplicates_removed: 1,
        }
    );
    // The uuid fallback supplies the dedupe key.
    assert_eq!(by_request(&records, "u7").usage.input, 10);
}

#[test]
fn cache_writes_use_the_ttl_breakdown_or_fall_back_to_5m() {
    let (records, stats) = read_all("cache-and-thinking", r"C:\repo");
    assert_eq!(stats.records_emitted, 4);
    assert_eq!(stats.skipped_total(), 0);

    let split = by_request(&records, "req_split").usage;
    assert_eq!((split.cache_write_5m, split.cache_write_1h), (25, 15));
    assert_eq!(split.input, 10);
    assert_eq!(split.output, 20);
    assert_eq!(split.cache_read, 30);
    assert_eq!(split.thinking, None);

    let flat = by_request(&records, "req_flat").usage;
    assert_eq!((flat.cache_write_5m, flat.cache_write_1h), (40, 0));
}

#[test]
fn thinking_is_informational_and_distinguishes_zero_from_absent() {
    let (records, _) = read_all("cache-and-thinking", r"C:\repo");
    let think = by_request(&records, "req_think").usage;
    assert_eq!(think.thinking, Some(7));
    assert_eq!(think.output, 20, "thinking is not added to output");
    assert_eq!(by_request(&records, "req_zero").usage.thinking, Some(0));
}

#[test]
fn discover_errors_when_the_projects_root_is_missing() {
    let missing = fixture("does-not-exist");
    let err = ClaudeCode
        .discover(&missing, &RepoIdentity::new(r"C:\repo"))
        .unwrap_err();
    assert!(matches!(err, SourceError::ProjectsRootMissing { .. }));
}

#[test]
fn read_errors_name_the_path_when_a_root_is_unreadable() {
    let gone = TranscriptRoot::new(fixture("does-not-exist"), RootKind::Repository);
    let err = ClaudeCode.read(&[gone], &mut |_| {}).unwrap_err();
    assert!(matches!(err, SourceError::Io { .. }));
    assert!(err.to_string().contains("does-not-exist"));
}

#[test]
fn invalid_utf8_and_unterminated_last_lines_are_handled() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("C--repo");
    std::fs::create_dir_all(&root).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\xff\xfe not json\n");
    bytes.extend_from_slice(
        br#"{"requestId":"r1","sessionId":"s","message":{"model":"m","usage":{"input_tokens":1,"output_tokens":2}}}"#,
    );
    std::fs::write(root.join("s.jsonl"), bytes).unwrap();

    let roots = ClaudeCode
        .discover(dir.path(), &RepoIdentity::new(r"C:\repo"))
        .unwrap();
    let mut records = Vec::new();
    let stats = ClaudeCode.read(&roots, &mut |r| records.push(r)).unwrap();
    assert_eq!(
        stats,
        ReadStats {
            files_read: 1,
            records_emitted: 1,
            malformed_lines: 1,
            ..ReadStats::default()
        }
    );
    assert_eq!(records[0].usage.output, 2);
    assert_eq!(records[0].usage.cache_write_5m, 0);
}
