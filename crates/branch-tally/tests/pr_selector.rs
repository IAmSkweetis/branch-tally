//! `report --pr`: the selector is a number, and every `gh` call is pinned to the origin repository
//! with `--repo` (#99). `gh` is faked with a shell script on `PATH` that logs its argv, so these
//! run offline; the fake is Unix-only.

#![expect(
    clippy::expect_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

#[cfg(unix)]
mod unix {
    use super::common::{World, stderr, stdout};
    use std::fs;

    /// Gives the world's repository an `origin` remote.
    fn add_origin(world: &World, url: &str) {
        let status = std::process::Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(&world.repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git remote add");
        assert!(status.success());
    }

    /// A fake `gh` that appends one line per call (its argv, space-joined) to `gh.log`, answers
    /// `pr list` with the given JSON and `pr view` with an empty body.
    struct FakeGh {
        dir: tempfile::TempDir,
    }

    impl FakeGh {
        fn new(list_json: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().expect("tempdir");
            let script = format!(
                "#!/bin/sh\necho \"$*\" >> \"{log}\"\ncase \"$2\" in\n  list) echo '{list_json}' ;;\n  view) echo '{{\"body\":\"\"}}' ;;\n  *) cat >/dev/null ;;\nesac\n",
                log = dir.path().join("gh.log").display()
            );
            let gh = dir.path().join("gh");
            fs::write(&gh, script).expect("write gh");
            fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("chmod");
            Self { dir }
        }

        fn same_repo() -> Self {
            Self::new(
                r#"[{"number":7,"headRepositoryOwner":{"login":"acme"},"isCrossRepository":false}]"#,
            )
        }

        fn path(&self) -> String {
            format!(
                "{}:{}",
                self.dir.path().display(),
                std::env::var("PATH").unwrap_or_default()
            )
        }

        fn calls(&self) -> Vec<String> {
            fs::read_to_string(self.dir.path().join("gh.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    fn run(world: &World, gh: &FakeGh, args: &[&str]) -> std::process::Output {
        let mut cmd = world.command(&world.repo, args);
        cmd.env("PATH", gh.path());
        cmd.output().expect("run")
    }

    const ORIGIN: &str = "https://github.com/acme/widget.git";

    #[test]
    fn pr_without_a_number_passes_repo_to_every_gh_call() {
        let world = World::scenario("subagent");
        add_origin(&world, ORIGIN);
        let gh = FakeGh::same_repo();
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "--dry-run"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let calls = gh.calls();
        assert!(calls.len() >= 2, "{calls:?}");
        for call in &calls {
            assert!(call.contains("--repo acme/widget"), "{call}");
        }
        assert!(calls.iter().any(|c| c.starts_with("pr list")), "{calls:?}");
        assert!(
            calls.iter().any(|c| c.starts_with("pr view 7")),
            "{calls:?}"
        );
    }

    #[test]
    fn pr_with_a_number_passes_repo_to_every_gh_call() {
        let world = World::scenario("subagent");
        add_origin(&world, ORIGIN);
        let gh = FakeGh::same_repo();
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "5", "--dry-run"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let calls = gh.calls();
        assert_ne!(calls, Vec::<String>::new());
        for call in &calls {
            assert!(call.contains("--repo acme/widget"), "{call}");
            assert!(!call.contains("pr list"), "{call}");
        }
        assert!(
            calls.iter().any(|c| c.starts_with("pr view 5")),
            "{calls:?}"
        );
    }

    #[test]
    fn pr_on_an_enterprise_host_passes_host_owner_repo() {
        let world = World::scenario("subagent");
        add_origin(&world, "https://ghe.example.com/acme/widget.git");
        let gh = FakeGh::same_repo();
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "5", "--dry-run"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let calls = gh.calls();
        assert_ne!(calls, Vec::<String>::new());
        for call in calls {
            assert!(
                call.contains("--repo ghe.example.com/acme/widget"),
                "{call}"
            );
        }
    }

    #[test]
    fn pr_selectors_that_are_not_a_number_are_usage_errors_and_never_run_gh() {
        let cases: [&[&str]; 6] = [
            &["--pr", "abc"],
            &["--pr", "https://github.com/o/r/pull/1"],
            &["--pr=--repo=x"],
            &["--pr", "-1"],
            &["--pr", "feat/x"],
            &["--pr", "0"],
        ];
        for extra in cases {
            let world = World::scenario("subagent");
            add_origin(&world, ORIGIN);
            let gh = FakeGh::same_repo();
            let mut args = vec!["report", "feat/x", "--dry-run"];
            args.extend_from_slice(extra);
            let out = run(&world, &gh, &args);
            assert_eq!(out.status.code(), Some(2), "{extra:?}: {}", stderr(&out));
            assert_eq!(stdout(&out), "", "{extra:?}");
            assert_eq!(gh.calls(), Vec::<String>::new(), "{extra:?}");
        }
    }

    #[test]
    fn pr_without_an_origin_remote_exits_1_without_running_gh() {
        let world = World::scenario("subagent");
        let gh = FakeGh::same_repo();
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "5", "--dry-run"]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(stderr(&out).contains("origin"), "{}", stderr(&out));
        assert_eq!(gh.calls(), Vec::<String>::new());
    }

    #[test]
    fn pr_with_an_unparseable_origin_exits_1_without_echoing_the_url() {
        let world = World::scenario("subagent");
        add_origin(&world, "/srv/secret-token-path/repo.git");
        let gh = FakeGh::same_repo();
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "5", "--dry-run"]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(
            !stderr(&out).contains("secret-token-path"),
            "{}",
            stderr(&out)
        );
        assert_eq!(gh.calls(), Vec::<String>::new());
    }

    #[test]
    fn pr_with_only_fork_prs_reports_the_ignored_count_and_exits_1() {
        let world = World::scenario("subagent");
        add_origin(&world, ORIGIN);
        let gh = FakeGh::new(
            r#"[{"number":9,"headRepositoryOwner":{"login":"evil"},"isCrossRepository":true}]"#,
        );
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "--dry-run"]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("1 fork PR(s) ignored; pass --pr N"),
            "{}",
            stderr(&out)
        );
    }

    #[test]
    fn pr_with_several_matches_lists_the_numbers_and_exits_1() {
        let world = World::scenario("subagent");
        add_origin(&world, ORIGIN);
        let gh = FakeGh::new(
            r#"[{"number":12,"headRepositoryOwner":{"login":"acme"},"isCrossRepository":false},{"number":3,"headRepositoryOwner":{"login":"acme"},"isCrossRepository":false}]"#,
        );
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "--dry-run"]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(stderr(&out).contains("3, 12"), "{}", stderr(&out));
    }
}
