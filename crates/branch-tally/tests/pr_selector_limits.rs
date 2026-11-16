//! `report --pr` failure messages for an empty and a possibly truncated PR list (#99). `gh` is
//! faked with a shell script on `PATH`, so these run offline; the fake is Unix-only.

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
    use super::common::{World, stderr};
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
    fn pr_with_no_prs_at_all_omits_the_fork_parenthetical() {
        let world = World::scenario("subagent");
        add_origin(&world, ORIGIN);
        let gh = FakeGh::new("[]");
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "--dry-run"]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("No open PR found"),
            "{}",
            stderr(&out)
        );
        assert!(!stderr(&out).contains("fork PR"), "{}", stderr(&out));
    }

    #[test]
    fn pr_with_a_full_list_fails_as_possibly_truncated_and_names_pr_n() {
        let world = World::scenario("subagent");
        add_origin(&world, ORIGIN);
        let entry =
            r#"{"number":7,"headRepositoryOwner":{"login":"acme"},"isCrossRepository":false}"#;
        let list = format!("[{}]", vec![entry; 100].join(","));
        let gh = FakeGh::new(&list);
        let out = run(&world, &gh, &["report", "feat/x", "--pr", "--dry-run"]);
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("may be truncated"),
            "{}",
            stderr(&out)
        );
        assert!(stderr(&out).contains("--pr N"), "{}", stderr(&out));
        assert!(
            gh.calls().iter().all(|c| !c.contains("pr view")),
            "{:?}",
            gh.calls()
        );
    }
}
