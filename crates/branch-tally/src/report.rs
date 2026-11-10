//! The `report` subcommand: resolve, run core's pipeline, print, optionally update the PR.
//!
//! Errors are returned as the message to print; `main` writes it to stderr and exits 1.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};

use branch_tally_core::git::{Git, SystemGit, normalize_path};
use branch_tally_core::model::{Harness, Mode};
use branch_tally_core::pr::{PrError, PrOutcome, SystemGh, update_pr};
use branch_tally_core::render;
use branch_tally_core::report::{Options, Report, ReportError};
use branch_tally_core::sources::claude_code::{ClaudeCode, slug_candidates};
use branch_tally_core::sources::{ReadStats, RepoIdentity, Source, SourceError, TranscriptRoot};

use crate::cli::ReportArgs;
use crate::prices;
use crate::pyrepr::repr;

/// Runs the command. `Err` carries the one-line message for stderr.
pub(crate) fn run(args: &ReportArgs) -> Result<(), String> {
    // The same lookup the reference's `Path.home()` makes: HOME, or USERPROFILE on Windows.
    let home = std::env::home_dir().ok_or("Could not determine the home directory")?;
    let projects = home.join(".claude").join("projects");
    let cwd = normalize_path(&args.repo);

    let table = prices::resolve_table(&prices::Locations::for_repo(&cwd))?;

    let source = CheckedSource::default();
    let report = branch_tally_core::report::run(
        &SystemGit,
        &source,
        &projects,
        &cwd,
        args.branch.as_deref(),
        // The CLI Builder adds --timeline / --work-branch against `Options` next.
        &Options::new(if args.strict {
            Mode::Strict
        } else {
            Mode::Claim
        }),
        &table,
    )
    .map_err(|e| {
        if let ReportError::NoRecords { read_stats, .. } = &e {
            note_skipped(read_stats);
        }
        describe(&e, &source)
    })?;

    note_skipped(&report.read_stats);
    for (model, _) in report
        .costs
        .by_model
        .iter()
        .filter(|(_, cost)| cost.is_none())
    {
        // Governance II: the floor is stated, and the unpriced models are named (ids only).
        stderr(&format!(
            "warning: no price for {model} in the price table (as of {}); the total is a floor",
            report.costs.price_table.as_of
        ));
    }
    if report.worktrees_unavailable.is_some() {
        // The reference treats this as "no worktrees"; say so rather than do it silently.
        stderr("note: git worktree list was unavailable, so worktree recovery was skipped");
    }

    if args.json {
        stdout(&format!(
            "{}\n",
            render::json(&report.summary, &report.costs)
        ));
        return Ok(());
    }
    let markdown = render::markdown(&report.summary, &report.costs, !args.no_cost);
    match &args.pr {
        None => stdout(&format!("{}\n", render::wrap(&markdown))),
        Some(number) => update(&cwd, &report, &markdown, number, args.dry_run)?,
    }
    Ok(())
}

fn update(
    cwd: &Path,
    report: &Report,
    markdown: &str,
    number: &str,
    dry_run: bool,
) -> Result<(), String> {
    // gh runs in the main checkout, as the reference does after resolving it.
    let repo: PathBuf = SystemGit.main_checkout(cwd);
    let number = (!number.is_empty()).then_some(number);
    let outcome = update_pr(
        &SystemGh::new(repo),
        &report.branch,
        markdown,
        number,
        dry_run,
    )
    .map_err(|e| match e {
        PrError::NoOpenPr { branch } => format!(
            "No open PR found for branch {}. Open the PR first, then re-run with --pr.",
            repr(&branch)
        ),
        PrError::Gh(e) => e.to_string(),
    })?;
    match outcome {
        PrOutcome::UpToDate { number } => stderr(&format!("PR #{number} already up to date.")),
        PrOutcome::DryRun { number, body } => {
            stdout(&format!("{body}\n"));
            stderr(&format!("\n--- dry run: PR #{number} not modified ---"));
        }
        PrOutcome::Written { number } => {
            stderr(&format!("Token usage table written to PR #{number}."));
        }
    }
    Ok(())
}

/// Governance II: say what the read could not use. One stderr line with counts only (never
/// content or paths), so stdout stays byte-identical to the reference.
///
/// Deduplicated replays are left out on purpose: a resumed session replays its history, the
/// reference removes those the same way, and announcing something that happens on nearly every
/// real run would teach people to ignore the note. Malformed lines, records without a request id
/// and synthetic records are the ones the reference drops without a word.
fn note_skipped(stats: &ReadStats) {
    let parts = [
        (stats.malformed_lines, "malformed"),
        (stats.no_dedupe_key, "without a request id"),
        (stats.synthetic, "synthetic"),
    ];
    let total: u64 = parts.iter().map(|(n, _)| n).sum();
    if total == 0 {
        return;
    }
    let detail: Vec<String> = parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
    stderr(&format!(
        "note: skipped {total} transcript record(s): {}",
        detail.join(", ")
    ));
}

/// The message for a failed report. Carries paths, branch names and git/IO errors, never
/// transcript content.
fn describe(error: &ReportError, source: &CheckedSource) -> String {
    match error {
        ReportError::NoRecords { branch, .. } => {
            if let Some((repo, slug)) = source.no_directory.borrow().as_ref() {
                return format!(
                    "No transcript directory for {} (expected slug {slug})",
                    repo.display()
                );
            }
            format!(
                "No transcript records found for branch {}. Transcripts are local and purged \
                 after cleanupPeriodDays (default 30).",
                repr(branch)
            )
        }
        ReportError::Source(SourceError::ProjectsRootMissing { path }) => {
            format!("No Claude Code transcript directory at {}", path.display())
        }
        other => other.to_string(),
    }
}

/// Wraps the Claude Code source to notice when no transcript directory matches the repository,
/// which the reference reports differently from "no records on this branch".
#[derive(Default)]
struct CheckedSource {
    inner: ClaudeCode,
    /// The repository and the slug looked for, set when discovery found no directory.
    no_directory: RefCell<Option<(PathBuf, String)>>,
}

impl Source for CheckedSource {
    fn harness(&self) -> Harness {
        self.inner.harness()
    }

    fn discover(
        &self,
        projects_root: &Path,
        repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError> {
        let roots = self.inner.discover(projects_root, repo)?;
        if roots.is_empty() {
            let slug = slug_candidates(repo.root())
                .into_iter()
                .next()
                .unwrap_or_default()
                .to_lowercase();
            *self.no_directory.borrow_mut() = Some((repo.root().to_path_buf(), slug));
        }
        Ok(roots)
    }

    fn read(
        &self,
        roots: &[TranscriptRoot],
        sink: &mut dyn FnMut(branch_tally_core::model::Record),
    ) -> Result<ReadStats, SourceError> {
        self.inner.read(roots, sink)
    }

    fn dispatches(
        &self,
        roots: &[TranscriptRoot],
    ) -> Result<Vec<branch_tally_core::model::Dispatch>, SourceError> {
        self.inner.dispatches(roots)
    }
}

/// Writes UTF-8 bytes verbatim: no newline translation, whatever the console codepage.
fn stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn stderr(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}
