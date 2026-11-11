//! The `report` and `record` subcommands: resolve, run core's pipeline, print, optionally update the PR.
//!
//! Errors are returned as the message to print; `main` writes it to stderr and exits 1.

use std::io::Write;
use std::path::{Path, PathBuf};

use branch_tally_core::config::{self, Flags, Format};
use branch_tally_core::git::{Git, SystemGit, normalize_path};
use branch_tally_core::ledger::{Identity, SystemClock};
use branch_tally_core::model::Mode;
use branch_tally_core::pr::{PrError, PrOutcome, SystemGh, update_pr};
use branch_tally_core::render;
use branch_tally_core::report::{
    FullSetCounts, LedgerReport, Options, Report, ReportError, WorkBranch,
};
use branch_tally_core::sources::claude_code::ClaudeCode;
use branch_tally_core::sources::{ReadStats, Source, SourceError};

use clap::CommandFactory;
use clap::error::ErrorKind;

use crate::cli::{Cli, RecordArgs, ReportArgs, WorkBranchPattern, parse_work_branch};
use crate::config::SystemEnv;
use crate::prices;
use crate::pyrepr::repr;

/// What a subcommand resolves before it reads anything: the effective settings, the work-branch
/// rule and the repository path.
struct Resolved {
    settings: config::Settings,
    pattern: Option<WorkBranchPattern>,
    cwd: PathBuf,
}

fn resolve(
    repo: &Path,
    config_path: Option<&Path>,
    flags: &Flags,
    work_branch_flag: Option<&WorkBranchPattern>,
) -> Result<Resolved, String> {
    let settings = config::load(&SystemEnv, config_path, flags).map_err(|e| e.to_string())?;
    if work_branch_flag.is_some() && settings.mode.value != Mode::Timeline {
        // A flag that cannot apply is a usage error; a configured one is just ignored.
        Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                "--work-branch needs timeline mode: pass --timeline or --mode timeline, or set `mode = \"timeline\"` in the config file",
            )
            .exit();
    }
    let pattern = work_branch_pattern(&settings)?;
    Ok(Resolved {
        settings,
        pattern,
        cwd: normalize_path(repo),
    })
}

fn projects_root() -> Result<PathBuf, String> {
    // The same lookup the reference's `Path.home()` makes: HOME, or USERPROFILE on Windows.
    let home = std::env::home_dir().ok_or("Could not determine the home directory")?;
    Ok(home.join(".claude").join("projects"))
}

/// The ledger file: `--ledger`, else `.tally/ledger.jsonl` under the checkout path.
fn ledger_path(flag: Option<&Path>, cwd: &Path) -> PathBuf {
    flag.map_or_else(
        || cwd.join(".tally").join("ledger.jsonl"),
        Path::to_path_buf,
    )
}

/// Runs the command. `Err` carries the one-line message for stderr.
pub(crate) fn run(args: &ReportArgs) -> Result<(), String> {
    let projects = projects_root()?;
    let Resolved {
        settings,
        pattern,
        cwd,
    } = resolve(
        &args.repo,
        args.config.as_deref(),
        &Flags {
            mode: args.mode_flag(),
            work_branch: args.work_branch.as_ref().map(|p| p.text.clone()),
            format: args.format_flag(),
            cost: args.cost_flag(),
        },
        args.work_branch.as_ref(),
    )?;
    let mode = settings.mode.value;

    let table = prices::resolve_table(&prices::Locations::for_repo(&cwd))?;

    let matches = |branch: &str| args.work_branch.as_ref().is_some_and(|p| p.matches(branch));
    let options = options(mode, pattern.as_ref(), &matches);
    let result = if args.from_ledger {
        let source: Option<&dyn Source> = args.with_live.then_some(&ClaudeCode as &dyn Source);
        branch_tally_core::report::run_from_ledger(
            &SystemGit,
            source,
            &projects,
            &cwd,
            args.branch.as_deref(),
            &options,
            &ledger_path(args.ledger.as_deref(), &cwd),
            &table,
        )
    } else {
        branch_tally_core::report::run(
            &SystemGit,
            &ClaudeCode,
            &projects,
            &cwd,
            args.branch.as_deref(),
            &options,
            &table,
        )
    };
    let report = result.map_err(|e| {
        if let ReportError::NoRecords { read_stats, .. } = &e {
            note_skipped(read_stats);
        }
        describe(&e, args.from_ledger)
    })?;

    if let Some(ledger) = &report.ledger {
        note_ledger(ledger);
    }
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

    if settings.format.value == Format::Json {
        stdout(&format!(
            "{}\n",
            render::json(&report.summary, &report.costs)
        ));
        return Ok(());
    }
    let markdown = render::markdown(&report.summary, &report.costs, settings.cost.value);
    match &args.pr {
        None => stdout(&format!("{}\n", render::wrap(&markdown))),
        Some(number) => update(&cwd, &report, &markdown, number, args.dry_run)?,
    }
    Ok(())
}

/// `record`: append the branch's new records to the ledger. Write only: no git add, no commit.
pub(crate) fn record(args: &RecordArgs) -> Result<(), String> {
    let projects = projects_root()?;
    let Resolved {
        settings,
        pattern,
        cwd,
    } = resolve(
        &args.repo,
        args.config.as_deref(),
        &Flags {
            mode: args.mode_flag(),
            work_branch: args.work_branch.as_ref().map(|p| p.text.clone()),
            format: None,
            cost: None,
        },
        args.work_branch.as_ref(),
    )?;
    let matches = |branch: &str| args.work_branch.as_ref().is_some_and(|p| p.matches(branch));
    let recorded = branch_tally_core::report::record(
        &SystemGit,
        &ClaudeCode,
        &projects,
        &cwd,
        args.branch.as_deref(),
        &options(settings.mode.value, pattern.as_ref(), &matches),
        &ledger_path(args.ledger.as_deref(), &cwd),
        &Identity,
        &SystemClock,
    )
    .map_err(|e| {
        if let ReportError::NoRecords { read_stats, .. } = &e {
            note_skipped(read_stats);
        }
        describe(&e, false)
    })?;
    note_skipped(&recorded.read_stats);
    if recorded.worktrees_unavailable.is_some() {
        stderr("note: git worktree list was unavailable, so worktree recovery was skipped");
    }
    stdout(&format!(
        "appended {}, skipped {}, branch {}\n",
        recorded.outcome.appended, recorded.outcome.skipped, recorded.branch
    ));
    Ok(())
}

/// The core options for the effective mode, with the work-branch rule in timeline mode.
fn options<'a>(
    mode: Mode,
    pattern: Option<&'a WorkBranchPattern>,
    matches: &'a dyn Fn(&str) -> bool,
) -> Options<'a> {
    let options = Options::new(mode);
    match (mode, pattern) {
        (Mode::Timeline, Some(p)) => options.with_work_branch(WorkBranch::Custom {
            pattern: &p.text,
            matches,
        }),
        _ => options,
    }
}

/// The active work-branch rule. One from a flag is already compiled; one from the config file is
/// compiled here, and a bad one names the file and the key (never the pattern).
fn work_branch_pattern(settings: &config::Settings) -> Result<Option<WorkBranchPattern>, String> {
    let (true, Some(text)) = (settings.work_branch_active, &settings.work_branch.value) else {
        return Ok(None);
    };
    parse_work_branch(text).map(Some).map_err(|_| {
        let file = settings
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        format!("config file {file}: key `work-branch` is not a valid regular expression")
    })
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

/// Governance II for a ledger-backed report: what the ledger could not tell. Counts only.
///
/// Duplicates are left out like replays are: a union merge or a second `record` makes them
/// routine, and the reader already collapsed them.
fn note_ledger(ledger: &LedgerReport) {
    if ledger.full_set == FullSetCounts::Unavailable {
        stderr("note: unattributed counts unavailable from the ledger");
    }
    if ledger.modes.len() > 1 {
        let names: Vec<&str> = ledger.modes.iter().map(|m| m.as_str()).collect();
        stderr(&format!(
            "note: ledger mixes modes ({}); the numbers combine them",
            names.join(", ")
        ));
    }
    let stats = &ledger.ledger_stats;
    let skipped = [
        (stats.malformed, "malformed"),
        (stats.unsupported_version, "unsupported version"),
    ];
    let total: u64 = skipped.iter().map(|(n, _)| n).sum();
    if total > 0 {
        let detail: Vec<String> = skipped
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{n} {what}"))
            .collect();
        stderr(&format!(
            "note: skipped {total} ledger line(s): {}",
            detail.join(", ")
        ));
    }
    if stats.conflicts > 0 {
        stderr(&format!(
            "note: {} ledger request(s) had conflicting lines; kept the latest",
            stats.conflicts
        ));
    }
    if let Some(live) = &ledger.live_read_stats {
        note_skipped(live);
    }
}

/// The message for a failed report. Carries paths, branch names and git/IO errors, never
/// transcript content.
fn describe(error: &ReportError, from_ledger: bool) -> String {
    match error {
        ReportError::NoRecords { branch, .. } if from_ledger => format!(
            "No ledger records found for branch {}. Run `branch-tally record` on a machine \
             that has the transcripts, or add --with-live.",
            repr(branch)
        ),
        ReportError::NoRecords { branch, .. } => format!(
            "No transcript records found for branch {}. Transcripts are local and purged \
             after cleanupPeriodDays (default 30).",
            repr(branch)
        ),
        ReportError::Source(SourceError::ProjectsRootMissing { path }) => {
            format!("No Claude Code transcript directory at {}", path.display())
        }
        other => other.to_string(),
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
