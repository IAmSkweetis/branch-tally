//! The Claude Code source, exercised against the hand-written fixtures in
//! `tests/fixtures/claude-code/`. Each scenario directory acts as the projects root.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers outside #[test] fns fail loudly on bad fixtures"
)]

use std::path::{Path, PathBuf};

use branch_tally_core::model::{AgentId, Dispatch, Harness, Origin, Record, SessionId};
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
            fallback_key_used: 1,
        }
    );
    // The uuid fallback supplies the dedupe key.
    assert_eq!(by_request(&records, "u7").usage.input, 10);
    assert!(by_request(&records, "u7").request_id_is_fallback);
    assert!(!by_request(&records, "req_1").request_id_is_fallback);
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

// --- Attribution scenarios (#2, #6, #7). The source reports raw facts; these tests pin the
// facts that make each scenario what it is. Every record's input tokens are distinct, so a
// mis-assigned record changes a per-branch total.

fn on_branch<'a>(records: &'a [Record], branch: &str) -> Vec<&'a Record> {
    records
        .iter()
        .filter(|r| r.git_branch.as_deref() == Some(branch))
        .collect()
}

fn input_sum(records: &[&Record]) -> u64 {
    records.iter().map(|r| r.usage.input).sum()
}

fn clean_stats(files: u64, emitted: u64) -> ReadStats {
    ReadStats {
        files_read: files,
        records_emitted: emitted,
        ..ReadStats::default()
    }
}

fn of_agent<'a>(records: &'a [Record], agent: &str) -> Vec<&'a Record> {
    records
        .iter()
        .filter(|r| r.agent_id.as_ref().map(AgentId::as_str) == Some(agent))
        .collect()
}

fn parent_is_s1(r: &Record) -> bool {
    r.parent_session.as_ref().map(SessionId::as_str) == Some("s1")
}

#[test]
fn session_that_starts_on_main_then_cuts_a_work_branch() {
    let (records, stats) = read_all("main-then-branch", r"C:\repo");
    assert_eq!(stats, clean_stats(1, 5));
    let main = on_branch(&records, "main");
    let work = on_branch(&records, "feat/new");
    assert_eq!((main.len(), input_sum(&main)), (2, 3));
    assert_eq!((work.len(), input_sum(&work)), (3, 12));
    assert!(records.iter().all(|r| r.session_id.as_str() == "s1"));
    // Timestamps are the only ordering signal and must survive untouched.
    assert_eq!(
        by_request(&records, "req_3").ts.as_deref(),
        Some("2026-01-01T00:02:00.000Z")
    );
}

#[test]
fn orchestrator_session_touches_several_work_branches() {
    let (records, stats) = read_all("orchestrator-multi-branch", r"C:\repo");
    assert_eq!(stats, clean_stats(1, 7));
    let sums: Vec<(&str, usize, u64)> = ["main", "feat/a", "feat/b", "refactor/c", "test/d"]
        .into_iter()
        .map(|b| {
            let v = on_branch(&records, b);
            (b, v.len(), input_sum(&v))
        })
        .collect();
    assert_eq!(
        sums,
        [
            ("main", 1, 1),
            ("feat/a", 2, 5),
            ("feat/b", 2, 10),
            ("refactor/c", 1, 5),
            ("test/d", 1, 7)
        ]
    );
    assert!(records.iter().all(|r| r.session_id.as_str() == "s1"));
}

#[test]
fn subagent_stamped_with_orchestrator_branch_while_cwd_is_a_worktree() {
    let (records, stats) = read_all("subagent-orchestrator-stamp", r"C:\repo");
    assert_eq!(stats, clean_stats(2, 4));
    let sub = by_request(&records, "req_b2");
    assert_eq!(sub.git_branch.as_deref(), Some("feat/x"));
    assert_eq!(
        sub.cwd.as_deref(),
        Some(r"C:\repo\.claude\worktrees\agent-b1")
    );
    assert!(sub.is_sidechain);
    assert_eq!(sub.agent_id.as_ref().map(AgentId::as_str), Some("b1"));
    assert!(parent_is_s1(sub));
    assert_eq!(input_sum(&on_branch(&records, "feat/x")), 10);
    assert_eq!(
        by_request(&records, "req_1").cwd.as_deref(),
        Some(r"C:\repo")
    );
}

#[test]
fn removed_worktree_subagents_keep_their_worktree_agent_stamp() {
    let (records, stats) = read_all("subagent-removed-worktree", r"C:\repo");
    assert_eq!(stats, clean_stats(4, 7));
    let subs: Vec<&Record> = records.iter().filter(|r| r.is_sidechain).collect();
    assert_eq!((subs.len(), input_sum(&subs)), (5, 25));
    for (agent, n, sum) in [("c1", 2, 7), ("c2", 1, 5), ("c3", 2, 13)] {
        let mine = of_agent(&records, agent);
        assert_eq!((mine.len(), input_sum(&mine)), (n, sum), "agent {agent}");
        let stamp = format!("worktree-agent-{agent}");
        assert!(
            mine.iter()
                .all(|r| r.git_branch.as_deref() == Some(&*stamp))
        );
        assert!(mine.iter().all(|r| parent_is_s1(r)));
    }
    assert_eq!(input_sum(&on_branch(&records, "feat/x")), 3);
}

#[test]
fn subagents_that_switch_to_their_own_branch_change_stamp_mid_file() {
    let (records, stats) = read_all("subagent-switches-branch", r"C:\repo");
    assert_eq!(stats, clean_stats(3, 8));
    let stamps = |agent: &str| -> Vec<(String, u64)> {
        let mut v = of_agent(&records, agent);
        v.sort_by_key(|r| r.usage.input);
        v.iter()
            .map(|r| (r.git_branch.clone().unwrap_or_default(), r.usage.input))
            .collect()
    };
    let want = |rows: &[(&str, u64)]| -> Vec<(String, u64)> {
        rows.iter().map(|(b, n)| ((*b).to_owned(), *n)).collect()
    };
    assert_eq!(
        stamps("d1"),
        want(&[
            ("worktree-agent-d1", 2),
            ("worktree-agent-d1", 3),
            ("chore/d1-data", 4),
            ("chore/d1-data", 5)
        ])
    );
    assert_eq!(
        stamps("d2"),
        want(&[
            ("worktree-agent-d2", 6),
            ("chore/d2-data", 7),
            ("chore/d2-data", 8)
        ])
    );
    assert_eq!(on_branch(&records, "feat/x").len(), 1);
}

#[test]
fn session_on_branch_a_edits_branch_b_through_a_second_worktree() {
    let (records, stats) = read_all("second-worktree-edit", r"C:\repo");
    assert_eq!(stats, clean_stats(1, 5));
    let wt = r"C:\repo\.claude\worktrees\feat-b";
    let in_wt: Vec<&Record> = records
        .iter()
        .filter(|r| r.cwd.as_deref() == Some(wt))
        .collect();
    assert_eq!(in_wt.len(), 3);
    // Two of the worktree records are still stamped with branch A: the stamp lags the cwd.
    let stamped_a: Vec<&Record> = in_wt
        .iter()
        .copied()
        .filter(|r| r.git_branch.as_deref() == Some("feat/a"))
        .collect();
    assert_eq!((stamped_a.len(), input_sum(&stamped_a)), (2, 7));
    assert_eq!(input_sum(&on_branch(&records, "feat/b")), 5);
    assert_eq!(input_sum(&on_branch(&records, "feat/a")), 10);
    assert!(records.iter().all(|r| r.session_id.as_str() == "s1"));
}

#[test]
fn records_are_tagged_with_the_root_they_were_read_from() {
    let (records, _) = read_all("worktree-and-siblings", r"C:\repo");
    assert_eq!(by_request(&records, "req_1").origin, Some(Origin::Main));
    assert_eq!(
        by_request(&records, "req_2").origin,
        Some(Origin::worktree("C--repo--claude-worktrees-feat-a"))
    );
    // Root names compare and label case-insensitively, like the reference's lowercased name.
    let wt_b = by_request(&records, "req_3").origin.clone().unwrap();
    assert_eq!(wt_b, Origin::worktree("c--repo--claude-worktrees-b"));
    assert_eq!(wt_b.label(), "c--repo--claude-worktrees-b");
}

#[test]
fn main_root_matching_the_slug_case_insensitively_is_main_origin() {
    let (records, _) = read_all("slug-case", r"C:\repo");
    assert_eq!(records.len(), 1);
    // The lowercase `c--repo` directory is the main checkout's root despite the case difference.
    assert_eq!(records[0].origin, Some(Origin::Main));
}

#[test]
fn subagent_files_under_the_main_root_are_main_origin() {
    // The reference tags by root, so a subagent file under the main slug is "main" (issue #6).
    let (records, _) = read_all("subagent", r"C:\repo");
    assert_eq!(by_request(&records, "req_a2").origin, Some(Origin::Main));
    assert!(records.iter().all(|r| r.origin == Some(Origin::Main)));
}

#[test]
fn files_are_read_in_one_path_sorted_pass_per_root() {
    let (records, stats) = read_all("read-order", r"C:\repo");
    assert_eq!(
        stats,
        ReadStats {
            files_read: 5,
            records_emitted: 4,
            duplicates_removed: 1,
            ..ReadStats::default()
        }
    );
    // `aaa/subagents/agent-x.jsonl` sorts before `zzz.jsonl`, so it wins the duplicate key.
    let dup = by_request(&records, "req_dup");
    assert_eq!(dup.git_branch.as_deref(), Some("from-sub"));
    assert_eq!(
        dup.parent_session.as_ref().map(SessionId::as_str),
        Some("aaa")
    );
}

#[test]
fn nested_non_subagent_files_are_read_like_main_files() {
    let (records, _) = read_all("read-order", r"C:\repo");
    let deep = by_request(&records, "req_deep");
    assert_eq!(deep.session_id.as_str(), "s9");
    assert_eq!(deep.parent_session, None);
    assert_eq!(deep.origin, Some(Origin::Main));
}

#[test]
fn worktree_roots_need_only_the_infix_prefix() {
    let roots = discover("read-order", r"C:\repo");
    assert_eq!(
        names(&roots),
        vec![
            ("C--repo".to_owned(), RootKind::Repository),
            ("C--repo--claude-worktrees".to_owned(), RootKind::Worktree),
            ("C--repo--claude-worktreesX".to_owned(), RootKind::Worktree),
        ]
    );
}

// ---- dispatch links (#6) -------------------------------------------------------------------

fn dispatches(scenario: &str) -> Vec<Dispatch> {
    let roots = discover(scenario, r"C:\repo");
    ClaudeCode.dispatches(&roots).unwrap()
}

type Link = (String, String, Option<String>);

/// `(session, agent, branch)` triples, sorted for a stable comparison.
fn links(scenario: &str) -> Vec<Link> {
    let mut found: Vec<Link> = dispatches(scenario)
        .into_iter()
        .map(|d| {
            (
                d.session_id.as_str().to_owned(),
                d.agent_id.as_str().to_owned(),
                d.git_branch,
            )
        })
        .collect();
    found.sort();
    found
}

fn link(session: &str, agent: &str, branch: Option<&str>) -> Link {
    (
        session.to_owned(),
        agent.to_owned(),
        branch.map(str::to_owned),
    )
}

#[test]
fn removed_worktree_scenario_links_all_three_subagents_to_the_orchestrator_branch() {
    assert_eq!(
        links("subagent-removed-worktree"),
        vec![
            link("s1", "c1", Some("feat/x")),
            link("s1", "c2", Some("feat/x")),
            link("s1", "c3", Some("feat/x")),
        ]
    );
    let all = dispatches("subagent-removed-worktree");
    assert!(all.iter().all(|d| d.ts.is_some()));
}

#[test]
fn dispatch_lines_do_not_change_what_read_reports() {
    let (_, stats) = read_all("subagent-removed-worktree", r"C:\repo");
    assert_eq!(stats, clean_stats(4, 7));
}

#[test]
fn every_subagent_scenario_has_its_dispatch_lines() {
    assert_eq!(links("subagent"), vec![link("s1", "a1", Some("feat/x"))]);
    assert_eq!(
        links("subagent-orchestrator-stamp"),
        vec![link("s1", "b1", Some("feat/x"))]
    );
    assert_eq!(
        links("subagent-switches-branch"),
        vec![
            link("s1", "d1", Some("feat/x")),
            link("s1", "d2", Some("feat/x"))
        ]
    );
    assert_eq!(
        links("subagent-renamed-branch"),
        vec![link("s1", "e1", Some("feat/x"))]
    );
    assert_eq!(
        links("subagent-shared-worktree"),
        vec![
            link("s1", "h1", Some("feat/x")),
            link("s1", "h2", Some("feat/x"))
        ]
    );
    assert_eq!(
        links("subagent-main-checkout"),
        vec![link("s1", "i1", Some("feat/x"))]
    );
}

#[test]
fn a_subagent_file_without_a_dispatch_line_has_no_link() {
    assert_eq!(
        links("subagent-no-dispatch-link"),
        vec![link("s1", "n1", Some("feat/x"))]
    );
    let (records, stats) = read_all("subagent-no-dispatch-link", r"C:\repo");
    assert_eq!(stats, clean_stats(3, 4));
    assert_eq!(of_agent(&records, "n2").len(), 2);
}

#[test]
fn dispatches_inside_subagent_files_and_stampless_dispatches_are_reported_raw() {
    assert_eq!(
        links("subagent-dispatcher-unusable"),
        vec![
            link("s1", "g1", Some("feat/x")),
            link("s1", "g2", Some("worktree-agent-g1")),
            link("s2", "g3", None),
        ]
    );
}

#[test]
fn malformed_and_non_agent_tool_results_are_skipped_and_replays_deduped() {
    // A string result, an object with no agentId, an empty agentId, a non-string agentId, a
    // truncated line and `42` yield nothing; the one real dispatch, replayed, is reported once.
    assert_eq!(
        links("malformed-and-keyless"),
        vec![link("s1", "m1", Some("main"))]
    );
    let (_, stats) = read_all("malformed-and-keyless", r"C:\repo");
    assert_eq!((stats.malformed_lines, stats.records_emitted), (2, 2));
}
