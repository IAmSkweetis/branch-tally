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
        number: String,
    },
    /// Dry run: the body that would be written.
    DryRun {
        /// The PR number.
        number: String,
        /// The updated body, LF line endings.
        body: String,
    },
    /// The body was written.
    Written {
        /// The PR number.
        number: String,
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
/// [`PrError`] when no PR is found or a gh call fails.
pub fn update_pr(
    gh: &dyn Gh,
    branch: &str,
    report: &str,
    number: Option<&str>,
    dry_run: bool,
) -> Result<PrOutcome, PrError> {
    let number = match number {
        Some(n) => n.to_owned(),
        None => find_open_pr(gh, branch)?,
    };
    let viewed = run_json(gh, &["pr", "view", &number, "--json", "body"], None)?;
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
    gh.run(&["pr", "edit", &number, "--body-file", "-"], Some(&updated))?;
    Ok(PrOutcome::Written { number })
}

fn find_open_pr(gh: &dyn Gh, branch: &str) -> Result<String, PrError> {
    let args = [
        "pr", "list", "--head", branch, "--state", "open", "--json", "number", "--limit", "1",
    ];
    let found = run_json(gh, &args, Some("[]"))?;
    let number = found
        .as_array()
        .ok_or_else(|| bad_output(&args))?
        .first()
        .map(|first| first.get("number").and_then(serde_json::Value::as_u64));
    match number {
        None => Err(PrError::NoOpenPr {
            branch: branch.to_owned(),
        }),
        Some(Some(n)) => Ok(n.to_string()),
        Some(None) => Err(bad_output(&args).into()),
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
