//! Asking git about branches and worktrees, behind a trait so tests inject a fake.
//!
//! Ports `worktree_branches` and the current-branch lookup in `main` from
//! `reference/token_report.py`. This module reports what git says and nothing more: building
//! Claude Code slugs or `worktree-<name>` keys from the result is attribution's job.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One git worktree that is on a branch.
///
/// Detached and bare worktrees have no branch and are never represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Resolved path of the worktree, without a Windows verbatim (`\\?\`) prefix.
    pub path: PathBuf,
    /// Branch name with `refs/heads/` removed.
    pub branch: String,
}

/// Why a git query failed. Messages carry the subcommand and exit status only.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The `git` executable could not be started (not installed, not on `PATH`, bad cwd).
    #[error("could not run `git {subcommand}`: {source}")]
    Spawn {
        /// The subcommand that was attempted.
        subcommand: &'static str,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// git ran and exited unsuccessfully (not a repository, for example).
    #[error("`git {subcommand}` failed (exit status {code})", code = display_code(*.code))]
    Failed {
        /// The subcommand that failed.
        subcommand: &'static str,
        /// Exit status, if the process was not killed by a signal.
        code: Option<i32>,
    },
    /// git succeeded but printed output that could not be read as the requested value.
    #[error("`git {subcommand}` printed output that could not be interpreted")]
    UnexpectedOutput {
        /// The subcommand whose output was unusable.
        subcommand: &'static str,
    },
}

fn display_code(code: Option<i32>) -> String {
    code.map_or_else(|| "none: killed by signal".to_owned(), |c| c.to_string())
}

/// Everything the report needs from git.
pub trait Git {
    /// The branch checked out in `dir` (`git rev-parse --abbrev-ref HEAD`).
    ///
    /// A detached HEAD yields the literal `HEAD`, as in the reference.
    ///
    /// # Errors
    ///
    /// [`GitError`] when the tool cannot run or `dir` is not in a repository.
    fn current_branch(&self, dir: &Path) -> Result<String, GitError>;

    /// The main checkout that owns `dir`'s repository: the parent of the resolved git common
    /// directory. Falls back to `dir` itself when git fails or prints nothing, as the
    /// reference does, so this is infallible.
    fn main_checkout(&self, dir: &Path) -> PathBuf;

    /// The branch-holding worktrees of the repository containing `repo`.
    ///
    /// Unlike the reference, which returns an empty map when git fails, a failure is an `Err`
    /// so the caller can report that worktree attribution was unavailable rather than silently
    /// reporting "no worktrees".
    ///
    /// # Errors
    ///
    /// [`GitError`] when the tool cannot run or `repo` is not in a repository.
    fn worktrees(&self, repo: &Path) -> Result<Vec<Worktree>, GitError>;

    /// The repository's default branch, as `origin` names it (`refs/remotes/origin/HEAD`), with
    /// the `origin/` prefix removed. Branch names containing `/` are kept whole.
    ///
    /// There is deliberately no guess: with no remote, or `origin/HEAD` unset, this is an `Err`
    /// and the caller decides on a fallback and says it used one.
    ///
    /// # Errors
    ///
    /// [`GitError`] when the tool cannot run, `repo` is not in a repository, or `origin/HEAD` is
    /// not resolvable ([`GitError::Failed`]), or git printed something unusable
    /// ([`GitError::UnexpectedOutput`]).
    fn default_branch(&self, repo: &Path) -> Result<String, GitError>;

    /// The configured `init.defaultBranch` (`git config --get init.defaultBranch`), the fallback
    /// when [`Git::default_branch`] cannot resolve `origin/HEAD`. It is a setting, not a fact
    /// about the repository, so a report that uses it must say so.
    ///
    /// # Errors
    ///
    /// [`GitError`] when the tool cannot run or the setting is unset ([`GitError::Failed`], exit
    /// status 1), or is empty ([`GitError::UnexpectedOutput`]).
    fn init_default_branch(&self, repo: &Path) -> Result<String, GitError>;
}

/// Extract the default branch from `git symbolic-ref --short refs/remotes/origin/HEAD` output
/// (`origin/main`). Pure. `None` when the output is empty or is not an `origin/` ref.
#[must_use]
pub fn parse_origin_head(output: &str) -> Option<String> {
    let name = output.trim().strip_prefix("origin/")?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

/// Parse `git worktree list --porcelain` output. Pure.
///
/// A `worktree <path>` line is paired with the `branch refs/heads/<b>` line that follows it
/// before the next entry. Detached, bare and non-`refs/heads` entries yield nothing. Paths are
/// run through [`normalize_path`]; porcelain prints forward slashes on Windows (`D:/x/y`).
#[must_use]
pub fn parse_worktree_porcelain(output: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(normalize_path(Path::new(p)));
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            if let Some(p) = path.take() {
                out.push(Worktree {
                    path: p,
                    branch: b.to_owned(),
                });
            }
        } else if line.is_empty() {
            path = None;
        }
    }
    out
}

/// Drop a Windows verbatim prefix via [`crate::paths::normalize`]. A non-UTF-8 path is returned
/// untouched, because normalising goes through lossy text and git must be handed the real bytes.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    if path.to_str().is_none() {
        return path.to_path_buf();
    }
    PathBuf::from(crate::paths::normalize(path))
}

/// Equivalent of Python's `Path.resolve()` as the reference uses it: canonicalize when the path
/// exists (following symlinks), otherwise keep it as given; never leave a verbatim prefix.
#[must_use]
pub fn normalize_path(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(p) => without_verbatim_prefix(&p),
        Err(_) => without_verbatim_prefix(path),
    }
}

/// The real implementation: shells out to the `git` on `PATH`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemGit;

impl SystemGit {
    fn run(dir: &Path, subcommand: &'static str, args: &[&str]) -> Result<String, GitError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|source| GitError::Spawn { subcommand, source })?;
        if !output.status.success() {
            return Err(GitError::Failed {
                subcommand,
                code: output.status.code(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Git for SystemGit {
    fn current_branch(&self, dir: &Path) -> Result<String, GitError> {
        let out = Self::run(
            dir,
            "rev-parse --abbrev-ref HEAD",
            &["rev-parse", "--abbrev-ref", "HEAD"],
        )?;
        Ok(out.trim().to_owned())
    }

    fn main_checkout(&self, dir: &Path) -> PathBuf {
        let common = Self::run(
            dir,
            "rev-parse --git-common-dir",
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .unwrap_or_default();
        let common = common.trim();
        if common.is_empty() {
            return dir.to_path_buf();
        }
        normalize_path(Path::new(common))
            .parent()
            .map_or_else(|| dir.to_path_buf(), Path::to_path_buf)
    }

    fn worktrees(&self, repo: &Path) -> Result<Vec<Worktree>, GitError> {
        let out = Self::run(
            repo,
            "worktree list --porcelain",
            &["worktree", "list", "--porcelain"],
        )?;
        Ok(parse_worktree_porcelain(&out))
    }

    fn default_branch(&self, repo: &Path) -> Result<String, GitError> {
        const SUBCOMMAND: &str = "symbolic-ref --short refs/remotes/origin/HEAD";
        let out = Self::run(
            repo,
            SUBCOMMAND,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )?;
        parse_origin_head(&out).ok_or(GitError::UnexpectedOutput {
            subcommand: SUBCOMMAND,
        })
    }

    fn init_default_branch(&self, repo: &Path) -> Result<String, GitError> {
        const SUBCOMMAND: &str = "config --get init.defaultBranch";
        let out = Self::run(repo, SUBCOMMAND, &["config", "--get", "init.defaultBranch"])?;
        let name = out.trim();
        if name.is_empty() {
            return Err(GitError::UnexpectedOutput {
                subcommand: SUBCOMMAND,
            });
        }
        Ok(name.to_owned())
    }
}

/// A scripted [`Git`] for tests, keyed by the directory passed in.
///
/// An unscripted directory behaves like a failure: `current_branch` and `worktrees` return
/// [`GitError::Failed`], and `main_checkout` falls back to the directory itself.
#[derive(Debug, Default, Clone)]
pub struct FakeGit {
    branches: BTreeMap<PathBuf, String>,
    mains: BTreeMap<PathBuf, PathBuf>,
    worktrees: BTreeMap<PathBuf, Vec<Worktree>>,
    default_branches: BTreeMap<PathBuf, String>,
    init_default_branches: BTreeMap<PathBuf, String>,
}

impl FakeGit {
    /// An empty fake: every directory is unscripted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script `current_branch(dir)`.
    #[must_use]
    pub fn with_branch(mut self, dir: impl Into<PathBuf>, branch: impl Into<String>) -> Self {
        self.branches.insert(dir.into(), branch.into());
        self
    }

    /// Script `main_checkout(dir)`.
    #[must_use]
    pub fn with_main_checkout(mut self, dir: impl Into<PathBuf>, main: impl Into<PathBuf>) -> Self {
        self.mains.insert(dir.into(), main.into());
        self
    }

    /// Script `worktrees(repo)`.
    #[must_use]
    pub fn with_worktrees(mut self, repo: impl Into<PathBuf>, list: Vec<Worktree>) -> Self {
        self.worktrees.insert(repo.into(), list);
        self
    }

    /// Script `default_branch(repo)`.
    #[must_use]
    pub fn with_default_branch(
        mut self,
        repo: impl Into<PathBuf>,
        branch: impl Into<String>,
    ) -> Self {
        self.default_branches.insert(repo.into(), branch.into());
        self
    }

    /// Script `init_default_branch(repo)`.
    #[must_use]
    pub fn with_init_default_branch(
        mut self,
        repo: impl Into<PathBuf>,
        branch: impl Into<String>,
    ) -> Self {
        self.init_default_branches
            .insert(repo.into(), branch.into());
        self
    }
}

impl Git for FakeGit {
    fn current_branch(&self, dir: &Path) -> Result<String, GitError> {
        self.branches.get(dir).cloned().ok_or(GitError::Failed {
            subcommand: "rev-parse --abbrev-ref HEAD",
            code: Some(128),
        })
    }

    fn main_checkout(&self, dir: &Path) -> PathBuf {
        self.mains
            .get(dir)
            .cloned()
            .unwrap_or_else(|| dir.to_path_buf())
    }

    fn worktrees(&self, repo: &Path) -> Result<Vec<Worktree>, GitError> {
        self.worktrees.get(repo).cloned().ok_or(GitError::Failed {
            subcommand: "worktree list --porcelain",
            code: Some(128),
        })
    }

    fn default_branch(&self, repo: &Path) -> Result<String, GitError> {
        self.default_branches
            .get(repo)
            .cloned()
            .ok_or(GitError::Failed {
                subcommand: "symbolic-ref --short refs/remotes/origin/HEAD",
                code: Some(128),
            })
    }

    fn init_default_branch(&self, repo: &Path) -> Result<String, GitError> {
        self.init_default_branches
            .get(repo)
            .cloned()
            .ok_or(GitError::Failed {
                subcommand: "config --get init.defaultBranch",
                code: Some(1),
            })
    }
}

#[cfg(test)]
mod default_branch_tests {
    use super::*;

    #[test]
    fn origin_head_output_is_stripped_of_the_remote() {
        assert_eq!(parse_origin_head("origin/main\n"), Some("main".to_owned()));
        assert_eq!(
            parse_origin_head("origin/main\r\n"),
            Some("main".to_owned())
        );
        assert_eq!(
            parse_origin_head("origin/release/1.x\n"),
            Some("release/1.x".to_owned())
        );
    }

    #[test]
    fn unusable_origin_head_output_is_none() {
        assert_eq!(parse_origin_head(""), None);
        assert_eq!(parse_origin_head("origin/\n"), None);
        assert_eq!(parse_origin_head("upstream/main\n"), None);
        assert_eq!(parse_origin_head("main\n"), None);
    }

    #[test]
    fn fake_init_default_branch_scripted_and_unscripted() {
        let f = FakeGit::new().with_init_default_branch("/r", "trunk");
        assert_eq!(f.init_default_branch(Path::new("/r")).unwrap(), "trunk");
        assert!(f.init_default_branch(Path::new("/other")).is_err());
    }

    #[test]
    fn fake_default_branch_scripted_and_unscripted() {
        let f = FakeGit::new().with_default_branch("/r", "trunk");
        assert_eq!(f.default_branch(Path::new("/r")).unwrap(), "trunk");
        assert!(f.default_branch(Path::new("/other")).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_yields_branch_worktrees_only() {
        let out = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n\
                   worktree /repo/.claude/worktrees/agent-1\nHEAD def\nbranch refs/heads/feat/x\n\n\
                   worktree /detached\nHEAD 123\ndetached\n\n\
                   worktree /bare\nbare\n";
        let got = parse_worktree_porcelain(out);
        let branches: Vec<_> = got.iter().map(|w| w.branch.as_str()).collect();
        assert_eq!(branches, ["main", "feat/x"]);
    }

    #[test]
    fn detached_entry_does_not_lend_its_path_to_a_later_branch() {
        // No blank separators: the branch line must pair with its own entry only.
        let out = "worktree /a\ndetached\nworktree /b\nbranch refs/heads/b\n";
        let got = parse_worktree_porcelain(out);
        assert_eq!(got.len(), 1);
        assert!(got[0].path.ends_with("b"));
    }

    #[test]
    fn windows_forward_slash_paths_and_crlf_parse() {
        let out = "worktree D:/x/y\r\nHEAD abc\r\nbranch refs/heads/feat/z\r\n";
        let got = parse_worktree_porcelain(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].branch, "feat/z");
        let s = got[0].path.to_string_lossy().replace('\\', "/");
        assert_eq!(s, "D:/x/y");
    }

    #[test]
    fn non_heads_ref_yields_nothing() {
        let out = "worktree /a\nbranch refs/remotes/o/x\n";
        assert_eq!(parse_worktree_porcelain(out), []);
    }

    #[test]
    fn verbatim_prefix_is_stripped() {
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\D:\x\y")),
            PathBuf::from(r"D:\x\y")
        );
        assert_eq!(
            without_verbatim_prefix(Path::new(r"\\?\UNC\srv\share\x")),
            PathBuf::from(r"\\srv\share\x")
        );
        assert_eq!(
            without_verbatim_prefix(Path::new("/plain/path")),
            PathBuf::from("/plain/path")
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_is_passed_through_untouched() {
        use std::os::unix::ffi::OsStrExt;
        let p = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe/x"));
        assert_eq!(without_verbatim_prefix(p), p);
    }

    #[test]
    fn nonexistent_path_is_kept_not_an_error() {
        let p = Path::new("definitely/not/a/real/dir-xyz");
        assert_eq!(normalize_path(p), p);
    }

    #[test]
    fn fake_scripted_and_unscripted() {
        let f = FakeGit::new()
            .with_branch("/r", "main")
            .with_worktrees(
                "/r",
                vec![Worktree {
                    path: "/r".into(),
                    branch: "main".into(),
                }],
            )
            .with_main_checkout("/r/wt", "/r");
        assert_eq!(f.current_branch(Path::new("/r")).unwrap(), "main");
        assert_eq!(f.main_checkout(Path::new("/r/wt")), PathBuf::from("/r"));
        assert_eq!(
            f.main_checkout(Path::new("/other")),
            PathBuf::from("/other")
        );
        assert_eq!(f.worktrees(Path::new("/r")).unwrap().len(), 1);
        assert!(f.worktrees(Path::new("/other")).is_err());
        assert!(f.current_branch(Path::new("/other")).is_err());
    }
}
