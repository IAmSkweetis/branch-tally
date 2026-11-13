//! The `report` and `record` subcommands: resolve, run core's pipeline, print, optionally update the PR.
//!
//! Errors are returned as the message to print; `main` writes it to stderr and exits 1.

use std::io::Write;
use std::path::{Path, PathBuf};

use branch_tally_core::config::{self, Flags, Format};
use branch_tally_core::git::{Git, SystemGit, normalize_path};
use branch_tally_core::ledger::{
    CAPTURES_PATH, CaptureReadStats, Identity, LedgerTarget, SystemClock,
};
use branch_tally_core::model::Mode;
use branch_tally_core::pr::{PrError, PrOutcome, SystemGh, update_pr};
use branch_tally_core::redact::RedactOptions;
use branch_tally_core::render;
use branch_tally_core::report::{
    FullSetCounts, LedgerReport, Options, Report, ReportError, WorkBranch,
};
use branch_tally_core::sources::claude_code::ClaudeCode;
use branch_tally_core::sources::{ReadStats, Source, SourceError};
use branch_tally_core::text::replace_control_chars;

use clap::CommandFactory;
use clap::error::ErrorKind;

use crate::cli::{Cli, RecordArgs, ReportArgs, WorkBranchPattern, parse_work_branch};
use crate::config::SystemEnv;
use crate::prices;
use crate::pyrepr::repr;

/// What a subcommand resolves before it reads anything: the effective settings, the work-branch
/// rule and the repository path.
pub(crate) struct Resolved {
    pub(crate) settings: config::Settings,
    pattern: Option<WorkBranchPattern>,
    pub(crate) cwd: PathBuf,
}

pub(crate) fn resolve(
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

/// The ledger a command uses: `--ledger` as given (the user's own choice, read and written as
/// is), else the guarded default `.tally/ledger.jsonl` at the checkout's top level, which core
/// opens without following links and never creates (#94).
pub(crate) fn ledger_target(flag: Option<&Path>) -> LedgerTarget<'_> {
    flag.map_or(LedgerTarget::Default, LedgerTarget::Explicit)
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
            redact: args.redact_flag(),
            redact_names: args.redact_names_flag(),
        },
        args.work_branch.as_ref(),
    )?;
    let mode = settings.mode.value;

    let table = prices::resolve_table(&prices::Locations::for_repo(&cwd))?;

    let matches = |branch: &str| args.work_branch.as_ref().is_some_and(|p| p.matches(branch));
    let options = options(
        mode,
        pattern.as_ref(),
        &matches,
        settings.redact.value.then_some(RedactOptions {
            names: settings.redact_names.value,
        }),
    );
    let result = if args.from_ledger {
        let source: Option<&dyn Source> = args.with_live.then_some(&ClaudeCode as &dyn Source);
        branch_tally_core::report::run_from_ledger_at(
            &SystemGit,
            source,
            &projects,
            &cwd,
            args.branch.as_deref(),
            &options,
            &ledger_target(args.ledger.as_deref()),
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
    note_captures(&report.capture_stats);
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
            redact: None,
            redact_names: None,
        },
        args.work_branch.as_ref(),
    )?;
    let matches = |branch: &str| args.work_branch.as_ref().is_some_and(|p| p.matches(branch));
    let recorded = branch_tally_core::report::record_at(
        &SystemGit,
        &ClaudeCode,
        &projects,
        &cwd,
        args.branch.as_deref(),
        &options(settings.mode.value, pattern.as_ref(), &matches, None),
        &ledger_target(args.ledger.as_deref()),
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
    note_captures(&recorded.capture_stats);
    if recorded.outcome.tally_absent {
        // Not an error: the repository has not opted in, so nothing was written or created.
        stdout("no .tally/ directory: nothing recorded (create it to opt in)\n");
        return Ok(());
    }
    if recorded.worktrees_unavailable.is_some() {
        stderr("note: git worktree list was unavailable, so worktree recovery was skipped");
    }
    if let Some(note) = unnamed_origins_note(recorded.outcome.unnamed_origins) {
        stderr(&note);
    }
    stdout(&format!(
        "appended {}, skipped {}, branch {}\n",
        recorded.outcome.appended, recorded.outcome.skipped, recorded.branch
    ));
    Ok(())
}

/// Governance II for `record`: entries whose session ran in a worktree outside the main checkout
/// are stored without a worktree name (the ledger never stores a path). Count only.
fn unnamed_origins_note(count: u64) -> Option<String> {
    (count > 0).then(|| {
        format!(
            "note: {count} record(s) from a worktree outside the main checkout were stored \
             without a worktree name"
        )
    })
}

/// The core options for the effective mode, with the work-branch rule in timeline mode.
fn options<'a>(
    mode: Mode,
    pattern: Option<&'a WorkBranchPattern>,
    matches: &'a dyn Fn(&str) -> bool,
    redaction: Option<RedactOptions>,
) -> Options<'a> {
    let mut options = Options::new(mode);
    if let Some(redaction) = redaction {
        options = options.with_redaction(redaction);
    }
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
    note_undated(stats.malformed_timestamps);
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

/// Governance II: records whose timestamp was not strict RFC 3339 are kept but undated. Informational,
/// not part of the skipped total. Count only.
fn note_undated(count: u64) {
    if count > 0 {
        stderr(&format!(
            "note: {count} record(s) had an unreadable timestamp; they were kept but are undated"
        ));
    }
}

/// Governance II for the hook captures: lines of `.tally/captures.jsonl` that could not be used.
/// Counts and the file's name only, never line content. Duplicates are left out: racing hooks and
/// union merges make them routine, and the reader already collapsed them.
fn note_captures(stats: &CaptureReadStats) {
    let skipped = [
        (stats.malformed, "malformed"),
        (stats.unsupported_version, "unsupported version"),
        (stats.unknown_kind, "unknown kind"),
    ];
    let total: u64 = skipped.iter().map(|(n, _)| n).sum();
    if total == 0 {
        return;
    }
    let detail: Vec<String> = skipped
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
    stderr(&format!(
        "note: skipped {total} capture line(s) in {CAPTURES_PATH}: {}",
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
    note_ledger_stats(&ledger.ledger_stats);
    if let Some(live) = &ledger.live_read_stats {
        note_skipped(live);
    }
}

/// Governance II: the ledger lines that could not be used, and conflicting requests. Counts only.
pub(crate) fn note_ledger_stats(stats: &branch_tally_core::ledger::ReadStats) {
    note_undated(stats.malformed_timestamps);
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
pub(crate) fn stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// The one choke point for stderr text. Messages interpolate recorded and ledger-derived text
/// (model ids, branch names, paths), so every line passes through
/// [`replace_control_chars`]: an escape sequence or bidi override in a transcript must not reach a
/// terminal. Newlines are the message's own layout, so the text is split on them first and each
/// line cleaned on its own.
pub(crate) fn clean_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| replace_control_chars(line).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn stderr(line: &str) {
    let _ = writeln!(std::io::stderr(), "{}", clean_lines(line));
}

#[cfg(test)]
mod tests {
    use super::unnamed_origins_note;

    #[test]
    fn unnamed_origins_note_is_silent_at_zero_and_counts_otherwise() {
        assert_eq!(unnamed_origins_note(0), None);
        assert_eq!(
            unnamed_origins_note(3).as_deref(),
            Some(
                "note: 3 record(s) from a worktree outside the main checkout were stored \
                 without a worktree name"
            )
        );
    }
}
