//! `ledger check`: the CLI half of the F-03 gate. Reads the committed `.tally/` files on the base
//! ref and on the working tree, hands them to core's pure check and prints each violation as file,
//! line and rule. Line text is never printed (Governance III).

use std::path::Path;

use branch_tally_core::git::{Git, SystemGit};
use branch_tally_core::ledger::{
    CheckInput, CheckedFile, Clock, MAX_TOKENS_PER_FIELD, Rule, SystemClock, TallyDir, Violation,
    check,
};
use branch_tally_core::model::Timestamp;

use crate::cli::LedgerCheckArgs;

pub(crate) fn run(args: &LedgerCheckArgs) -> Result<(), String> {
    let git = SystemGit;
    let top = git.toplevel(&args.repo).map_err(|e| {
        format!(
            "ledger check: {} is not inside a git checkout ({e})",
            args.repo.display()
        )
    })?;
    let head_branch = match &args.head_branch {
        Some(name) => name.clone(),
        None => git.current_branch(&top).map_err(|e| {
            format!("ledger check: cannot tell the current branch ({e}); pass --head-branch")
        })?,
    };
    let summary = check_repo(&git, &SystemClock, &top, &args.base, &head_branch)?;
    crate::report::stderr(&summary);
    Ok(())
}

/// Runs the check and returns the clean summary, or the violation listing as the error.
fn check_repo(
    git: &dyn Git,
    clock: &dyn Clock,
    top: &Path,
    base: &str,
    head_branch: &str,
) -> Result<String, String> {
    let base_file = |name: &str| {
        git.file_at(top, base, &Path::new(".tally").join(name))
            .map_err(|e| format!("ledger check: cannot read {name} at base '{base}': {e}"))
    };
    let base_ledger = base_file("ledger.jsonl")?;
    let base_captures = base_file("captures.jsonl")?;
    let tally = TallyDir::open(top).map_err(|e| format!("ledger check: {e}"))?;
    let head_file = |name: &str| -> Result<Option<Vec<u8>>, String> {
        match &tally {
            Some(t) => t.read(name).map_err(|e| format!("ledger check: {e}")),
            None => Ok(None),
        }
    };
    let head_ledger = head_file("ledger.jsonl")?;
    let head_captures = head_file("captures.jsonl")?;
    let now: Timestamp = clock
        .now()
        .parse()
        .map_err(|_| "ledger check: the clock gave an invalid time".to_owned())?;
    let report = check(&CheckInput {
        base_ledger: base_ledger.as_deref(),
        head_ledger: head_ledger.as_deref(),
        base_captures: base_captures.as_deref(),
        head_captures: head_captures.as_deref(),
        head_branch,
        now: &now,
    });
    if report.is_clean() {
        return Ok(format!("ledger check: clean against {base}"));
    }
    let mut lines: Vec<String> = report.violations.iter().map(describe).collect();
    lines.push(format!(
        "ledger check: {} violation(s) against {base}",
        report.violations.len()
    ));
    Err(lines.join("\n"))
}

fn describe(v: &Violation) -> String {
    let file = match v.file {
        CheckedFile::Ledger => "ledger",
        CheckedFile::Captures => "captures",
    };
    let place = match v.line {
        Some(n) => format!("line {n}"),
        None => "removed base line".to_owned(),
    };
    let rule = match v.rule {
        Rule::BaseLineRemoved => "a base line was removed or changed".to_owned(),
        Rule::Unparseable => "unparseable".to_owned(),
        Rule::WrongBranch => "wrong branch (not recorded for the head branch)".to_owned(),
        Rule::MissingRecordedAt => "missing recorded_at".to_owned(),
        Rule::InvalidTimestamp => "invalid timestamp".to_owned(),
        Rule::FutureTimestamp => "future timestamp".to_owned(),
        Rule::DuplicateKey => "duplicate of a base request".to_owned(),
        Rule::TokenOverCeiling { field } => {
            format!("token field {field} over the {MAX_TOKENS_PER_FIELD} ceiling")
        }
    };
    format!("{file}: {place}: {rule}")
}
