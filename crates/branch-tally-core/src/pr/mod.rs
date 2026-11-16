//! Splicing a report into a pull request body through `gh`, behind a trait so tests inject a fake.
//!
//! Ports `gh` and `update_pr` from `reference/token_report.py`.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// Why a `gh` call failed. Messages carry the arguments and gh's own output, never transcripts.
#[derive(Debug, thiserror::Error)]
pub enum GhError {
    /// The `gh` executable could not be started.
    #[error("could not run `gh`: {0}")]
    Spawn(#[source] std::io::Error),
    /// gh ran and exited unsuccessfully; `detail` is its stderr, else its stdout, trimmed.
    #[error("gh {args} failed: {detail}")]
    Failed {
        /// The arguments, space-joined.
        args: String,
        /// What gh printed.
        detail: String,
    },
    /// gh succeeded but its output was not the JSON shape asked for.
    #[error("gh {args} printed output that is not the expected JSON")]
    BadOutput {
        /// The arguments, space-joined.
        args: String,
    },
}

/// Why the PR could not be updated.
#[derive(Debug, thiserror::Error)]
pub enum PrError {
    /// No open PR has the branch as its head.
    #[error("no open PR found for branch {branch:?}")]
    NoOpenPr {
        /// The branch searched for.
        branch: String,
        /// Open PRs for the branch that were ignored (fork PRs); 0 when none.
        ignored: usize,
    },
    /// More than one open same-repository PR has the branch as its head.
    #[error("more than one open PR for branch {branch:?}: {numbers:?}")]
    AmbiguousPr {
        /// The branch searched for.
        branch: String,
        /// Every matching PR number, sorted.
        numbers: Vec<u64>,
    },
    /// A `gh` call failed.
    #[error(transparent)]
    Gh(#[from] GhError),
}

/// What [`update_pr`] did, for the caller to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrOutcome {
    /// The body already holds exactly this report.
    UpToDate {
        /// The PR number.
        number: u64,
    },
    /// Dry run: the body that would be written.
    DryRun {
        /// The PR number.
        number: u64,
        /// The updated body, LF line endings.
        body: String,
    },
    /// The body was written.
    Written {
        /// The PR number.
        number: u64,
    },
}

/// Everything the PR update needs from the GitHub CLI.
pub trait Gh {
    /// Runs `gh <args>`, with `stdin` (UTF-8) if given, and returns its stdout.
    ///
    /// # Errors
    ///
    /// [`GhError`] when gh cannot start or exits unsuccessfully.
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String, GhError>;
}

/// Splices `report` into the PR body, or reports why not (`update_pr`).
///
/// # Errors
///
/// [`PrError`] when no single PR is found or a gh call fails.
/// `repo` is `OWNER/REPO`, passed to every gh call as `--repo`.
pub fn update_pr(
    gh: &dyn Gh,
    branch: &str,
    report: &str,
    repo: &str,
    number: Option<u64>,
    dry_run: bool,
) -> Result<PrOutcome, PrError> {
    let number = match number {
        Some(n) => n,
        None => find_open_pr(gh, branch, repo)?,
    };
    let num = number.to_string();
    let viewed = run_json(
        gh,
        &["pr", "view", &num, "--repo", repo, "--json", "body"],
        None,
    )?;
    let body = viewed
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let updated = crate::render::splice(body, report);
    if updated.trim() == body.replace("\r\n", "\n").trim() {
        return Ok(PrOutcome::UpToDate { number });
    }
    if dry_run {
        return Ok(PrOutcome::DryRun {
            number,
            body: updated,
        });
    }
    gh.run(
        &["pr", "edit", &num, "--repo", repo, "--body-file", "-"],
        Some(&updated),
    )?;
    Ok(PrOutcome::Written { number })
}

/// The one open same-repository PR whose head is `branch`. Fork PRs are never chosen: they must be
/// named with an explicit number, and how many were skipped is reported.
fn find_open_pr(gh: &dyn Gh, branch: &str, repo: &str) -> Result<u64, PrError> {
    let args = [
        "pr",
        "list",
        "--repo",
        repo,
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,headRepositoryOwner,isCrossRepository",
        "--limit",
        "100",
    ];
    let found = run_json(gh, &args, Some("[]"))?;
    let entries = found.as_array().ok_or_else(|| bad_output(&args))?;
    let repo_owner = repo.split('/').next().unwrap_or_default();
    let mut kept = Vec::new();
    let mut ignored = 0_usize;
    for entry in entries {
        let number = entry
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| bad_output(&args))?;
        let cross = entry
            .get("isCrossRepository")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| bad_output(&args))?;
        // A fork whose repository was deleted has no owner; it is ignored either way.
        let owner = entry
            .get("headRepositoryOwner")
            .and_then(|o| o.get("login"))
            .and_then(serde_json::Value::as_str);
        if cross {
            ignored += 1;
            continue;
        }
        let owner = owner.ok_or_else(|| bad_output(&args))?;
        if owner.eq_ignore_ascii_case(repo_owner) {
            kept.push(number);
        } else {
            ignored += 1;
        }
    }
    kept.sort_unstable();
    match kept.as_slice() {
        [] => Err(PrError::NoOpenPr {
            branch: branch.to_owned(),
            ignored,
        }),
        [n] => Ok(*n),
        _ => Err(PrError::AmbiguousPr {
            branch: branch.to_owned(),
            numbers: kept,
        }),
    }
}

fn bad_output(args: &[&str]) -> GhError {
    GhError::BadOutput {
        args: args.join(" "),
    }
}

/// Runs gh and parses stdout as JSON. Empty stdout parses as `empty` when given (the reference
/// treats an empty PR list as `[]`).
fn run_json(gh: &dyn Gh, args: &[&str], empty: Option<&str>) -> Result<serde_json::Value, GhError> {
    let out = gh.run(args, None)?;
    let text = match empty {
        Some(e) if out.trim().is_empty() => e,
        _ => &out,
    };
    serde_json::from_str(text).map_err(|_| bad_output(args))
}

/// The real [`Gh`]: runs the `gh` on `PATH` with `dir` as its working directory.
#[derive(Debug, Clone)]
pub struct SystemGh {
    dir: std::path::PathBuf,
}

impl SystemGh {
    /// Runs gh inside `dir`, so it resolves the repository the way the reference does.
    #[must_use]
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Gh for SystemGh {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String, GhError> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut cmd = Command::new("gh");
        cmd.args(args)
            .current_dir(&self.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = cmd.spawn().map_err(GhError::Spawn)?;
        // The body goes in as UTF-8 bytes whatever the console codepage: the reference hung on
        // Windows when text mode encoded it as cp1252. A writer thread keeps a full pipe from
        // blocking against gh's own output. The pipe closes when the handle drops.
        let writer = child.stdin.take().zip(stdin).map(|(mut pipe, text)| {
            let bytes = text.as_bytes().to_vec();
            std::thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            })
        });
        let output = child.wait_with_output().map_err(GhError::Spawn)?;
        if let Some(w) = writer {
            let _ = w.join();
        }
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = match stderr.trim() {
                "" => stdout.trim().to_owned(),
                e => e.to_owned(),
            };
            return Err(GhError::Failed {
                args: args.join(" "),
                detail,
            });
        }
        Ok(stdout)
    }
}

/// One recorded [`FakeGh`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The arguments.
    pub args: Vec<String>,
    /// The stdin given, if any.
    pub stdin: Option<String>,
}

/// A scripted [`Gh`] for tests, keyed by the space-joined arguments. An unscripted call fails.
#[derive(Debug, Default)]
pub struct FakeGh {
    script: BTreeMap<String, Result<String, String>>,
    calls: RefCell<Vec<Call>>,
}

impl FakeGh {
    /// A fake with nothing scripted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script `args` to succeed with `stdout`, or (`Err`) to fail with that detail.
    #[must_use]
    pub fn on(mut self, args: &[&str], result: Result<&str, &str>) -> Self {
        self.script.insert(
            args.join(" "),
            result.map(str::to_owned).map_err(str::to_owned),
        );
        self
    }

    /// Every call made so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }
}

impl Gh for FakeGh {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String, GhError> {
        self.calls.borrow_mut().push(Call {
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            stdin: stdin.map(str::to_owned),
        });
        let key = args.join(" ");
        match self.script.get(&key) {
            Some(Ok(out)) => Ok(out.clone()),
            Some(Err(detail)) => Err(GhError::Failed {
                args: key,
                detail: detail.clone(),
            }),
            None => Err(GhError::Failed {
                args: key,
                detail: "unscripted call".to_owned(),
            }),
        }
    }
}
