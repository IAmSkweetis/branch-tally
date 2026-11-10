//! Shared harness: a throwaway home, a real git repository and a fixture scenario copied in under
//! that repository's real slug, as `scripts/make_goldens.py` does for the Python reference.
//!
//! The binary resolves `~/.claude/projects` from `HOME`/`USERPROFILE`, so a test points it at a
//! fixture by setting both. Nothing here touches the real home directory.

#![expect(clippy::expect_used, clippy::panic, reason = "test setup fails loudly")]

use std::collections::BTreeMap;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_branch-tally");

/// Where the shared fixtures live, relative to this crate.
pub(crate) fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude-code")
}

/// Claude Code's slug for a path, from core's shared `paths` module.
pub(crate) fn slug_for(path: &Path) -> String {
    branch_tally_core::paths::slug(path)
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=bt",
            "-c",
            "user.email=bt@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("these tests require `git` on PATH");
    assert!(out.status.success(), "git {args:?} failed");
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// A repository, a home with `.claude/projects`, and optionally a scenario copied into it.
pub(crate) struct World {
    _scratch: tempfile::TempDir,
    pub(crate) home: PathBuf,
    pub(crate) repo: PathBuf,
    /// Lowercased directory name the binary reports as a session origin, back to the fixture's own.
    pub(crate) origins: BTreeMap<String, String>,
}

impl World {
    /// An empty home and a repository on `main` with one commit. No transcripts.
    pub(crate) fn empty() -> Self {
        let scratch = tempfile::Builder::new()
            .prefix("bt-e2e-")
            .tempdir()
            .expect("tempdir");
        let root = canon(scratch.path());
        let home = root.join("home");
        let repo = root.join("work").join("repo");
        fs::create_dir_all(home.join(".claude").join("projects")).expect("projects");
        fs::create_dir_all(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        Self {
            _scratch: scratch,
            home,
            repo,
            origins: BTreeMap::new(),
        }
    }

    pub(crate) fn projects(&self) -> PathBuf {
        self.home.join(".claude").join("projects")
    }

    /// Builds the scenario's worktrees and copies its directories in under this repo's real slug.
    pub(crate) fn scenario(name: &str) -> Self {
        let mut world = Self::empty();
        let dir = fixtures_root().join(name);
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("goldens").join("scenario.json")).expect("manifest"),
        )
        .expect("manifest is JSON");
        let repo_slug = manifest["repo_slug"]
            .as_str()
            .expect("repo_slug")
            .to_owned();
        for wt in manifest["worktrees"].as_array().expect("worktrees") {
            let path = world
                .repo
                .join(".claude")
                .join("worktrees")
                .join(wt["name"].as_str().expect("name"));
            git(
                &world.repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    wt["branch"].as_str().expect("branch"),
                    &path.to_string_lossy(),
                ],
            );
        }
        let real = slug_for(&canon(&world.repo));
        let mut names: Vec<String> = fs::read_dir(&dir)
            .expect("scenario dir")
            .map(|e| e.expect("entry"))
            .filter(|e| e.file_type().expect("type").is_dir() && e.file_name() != "goldens")
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for original in names {
            let copied = if original
                .to_lowercase()
                .starts_with(&repo_slug.to_lowercase())
            {
                let suffix = &original[repo_slug.len()..];
                let exact = original[..repo_slug.len()] == repo_slug;
                format!(
                    "{}{suffix}",
                    if exact {
                        real.clone()
                    } else {
                        real.to_lowercase()
                    }
                )
            } else {
                original.clone()
            };
            copy_dir(&dir.join(&original), &world.projects().join(&copied));
            world.origins.insert(copied.to_lowercase(), original);
        }
        world
    }

    /// The per-user cache root the binary sees: `LOCALAPPDATA` and `XDG_CACHE_HOME` both point
    /// here, and macOS derives it from `HOME`, so a developer's real cache never leaks in.
    pub(crate) fn cache_root(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.home.join("Library").join("Caches")
        } else {
            self.home.join("cache-root")
        }
    }

    /// The config root the binary sees: `APPDATA` and `XDG_CONFIG_HOME` both point here, so a
    /// developer's real config never leaks into a test.
    pub(crate) fn config_root(&self) -> PathBuf {
        self.home.join("config-root")
    }

    /// Writes `text` to the default config location the binary will look at.
    pub(crate) fn write_default_config(&self, text: &str) -> PathBuf {
        let dir = self.config_root().join("branch-tally");
        fs::create_dir_all(&dir).expect("config dir");
        let path = dir.join("config.toml");
        fs::write(&path, text).expect("write config");
        path
    }

    /// Writes `text` to a named file outside the default location, for `--config`.
    pub(crate) fn write_config_file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.home.join(name);
        fs::write(&path, text).expect("write config");
        path
    }

    /// Runs the binary in the repository with this world's home.
    pub(crate) fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.repo, args)
    }

    pub(crate) fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("run branch-tally")
    }

    pub(crate) fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("LOCALAPPDATA", self.cache_root())
            .env("XDG_CACHE_HOME", self.cache_root())
            .env("APPDATA", self.config_root())
            .env("XDG_CONFIG_HOME", self.config_root())
            .env_remove("BRANCH_TALLY_PRICES_URL")
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }

    /// Replaces each non-main session origin with the fixture directory it came from.
    pub(crate) fn normalize(&self, payload: &mut Value) {
        let Some(sessions) = payload["sessions"].as_array_mut() else {
            return;
        };
        for s in sessions {
            let Some(origin) = s["origin"].as_str().filter(|o| *o != "main") else {
                continue;
            };
            let original = self
                .origins
                .get(origin)
                .unwrap_or_else(|| panic!("unexpected origin {origin}"))
                .clone();
            s["origin"] = Value::String(original);
        }
    }
}

/// The resolved path without a Windows verbatim (`\\?\`) prefix, which git rejects.
pub(crate) fn canon(path: &Path) -> PathBuf {
    let resolved = fs::canonicalize(path).expect("canonicalize");
    PathBuf::from(branch_tally_core::paths::normalize(&resolved))
}

pub(crate) fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("stdout is UTF-8")
}

pub(crate) fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).expect("stderr is UTF-8")
}
