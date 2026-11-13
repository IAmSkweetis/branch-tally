//! `ledger check`: the CLI half of the F-03 gate. Reads the committed `.tally/` files on the base
//! ref and on the head (a rev, or the working tree), hands them to core's pure check and prints
//! each violation as file, line and rule. Line text is never printed (Governance III).

use std::path::Path;

use branch_tally_core::git::{Git, SystemGit};
use branch_tally_core::ledger::{
    CheckInput, CheckedFile, Clock, MAX_TOKENS_PER_FIELD, Rule, SystemClock, TallyDir, Violation,
    check,
};
use branch_tally_core::model::Timestamp;
use branch_tally_core::text::replace_control_chars;

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
        // On a detached HEAD git prints the literal `HEAD`, which is not a branch.
        None => match git.current_branch(&top) {
            Ok(name) if !name.is_empty() && name != "HEAD" => name,
            _ => {
                return Err(
                    "ledger check: cannot tell the current branch (detached HEAD?); \
                            pass --head-branch"
                        .to_owned(),
                );
            }
        },
    };
    // No fallback to the `--base` text: in CI that is a SHA, and the rule would silently never
    // fire (Governance II).
    let base_branch = match &args.base_branch {
        Some(name) => name.clone(),
        None => git.default_branch(&top).map_err(|e| {
            format!("ledger check: cannot tell the base branch ({e}); pass --base-branch")
        })?,
    };
    let summary = check_repo(
        &git,
        &SystemClock,
        &top,
        &Revs {
            base: &args.base,
            head: args.head.as_deref(),
        },
        &head_branch,
        &base_branch,
    )?;
    crate::report::stderr(&summary);
    Ok(())
}

/// The base rev and, when given, the head rev (else the working tree is the head).
struct Revs<'a> {
    base: &'a str,
    head: Option<&'a str>,
}

/// A ledger-derived name made safe to print: control characters replaced, and a leading `::`
/// (a GitHub workflow command when it begins a line) escaped with a backslash.
fn printable(name: &str) -> String {
    let clean = replace_control_chars(name);
    if clean.starts_with("::") {
        format!("\\{clean}")
    } else {
        clean.into_owned()
    }
}

/// The head's three `.tally/` files, each absent as `None`.
struct HeadFiles {
    ledger: Option<Vec<u8>>,
    captures: Option<Vec<u8>>,
    prices: Option<Vec<u8>>,
}

/// Reads them from `rev` through git, or from the working tree with the symlink guard.
fn read_head(git: &dyn Git, top: &Path, rev: Option<&str>) -> Result<HeadFiles, String> {
    if let Some(rev) = rev {
        let at = |name: &str| {
            git.file_at(top, rev, &Path::new(".tally").join(name))
                .map_err(|e| format!("ledger check: cannot read {name} at head '{rev}': {e}"))
        };
        return Ok(HeadFiles {
            ledger: at("ledger.jsonl")?,
            captures: at("captures.jsonl")?,
            prices: at("prices.toml")?,
        });
    }
    let tally = TallyDir::open(top).map_err(|e| format!("ledger check: {e}"))?;
    let read = |name: &str| -> Result<Option<Vec<u8>>, String> {
        match &tally {
            Some(t) => t.read(name).map_err(|e| format!("ledger check: {e}")),
            None => Ok(None),
        }
    };
    Ok(HeadFiles {
        ledger: read("ledger.jsonl")?,
        captures: read("captures.jsonl")?,
        prices: read("prices.toml")?,
    })
}

/// Runs the check and returns the clean summary, or the violation listing as the error. Notes and
/// warnings come first in either case and never change the outcome.
fn check_repo(
    git: &dyn Git,
    clock: &dyn Clock,
    top: &Path,
    revs: &Revs<'_>,
    head_branch: &str,
    base_branch: &str,
) -> Result<String, String> {
    let base = revs.base;
    let base_file = |name: &str| {
        git.file_at(top, base, &Path::new(".tally").join(name))
            .map_err(|e| format!("ledger check: cannot read {name} at base '{base}': {e}"))
    };
    let base_ledger = base_file("ledger.jsonl")?;
    let base_captures = base_file("captures.jsonl")?;
    let base_prices = base_file("prices.toml")?;
    let head_files = read_head(git, top, revs.head)?;
    let now: Timestamp = clock
        .now()
        .parse()
        .map_err(|_| "ledger check: the clock gave an invalid time".to_owned())?;
    let report = check(&CheckInput {
        base_ledger: base_ledger.as_deref(),
        head_ledger: head_files.ledger.as_deref(),
        base_captures: base_captures.as_deref(),
        head_captures: head_files.captures.as_deref(),
        head_branch,
        base_branch,
        now: &now,
    });
    let mut lines: Vec<String> = report
        .other_branches
        .iter()
        .map(|(name, n)| {
            format!(
                "note: {n} new ledger line(s) recorded for branch {}",
                printable(name)
            )
        })
        .collect();
    if base_prices != head_files.prices {
        lines.push("warning: .tally/prices.toml differs from base".to_owned());
    }
    if report.is_clean() {
        lines.push(format!("ledger check: clean against {base}"));
        return Ok(lines.join("\n"));
    }
    lines.extend(report.violations.iter().map(describe));
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
        Rule::RecordedForBaseBranch => "recorded for the base branch".to_owned(),
        Rule::MissingRecordedAt => "missing recorded_at".to_owned(),
        Rule::InvalidTimestamp => "invalid timestamp".to_owned(),
        Rule::FutureTimestamp => "future timestamp".to_owned(),
        Rule::DuplicateKey => "duplicate of a base request".to_owned(),
        Rule::ConflictingNewKey => {
            "conflicts with an earlier new line for the same request".to_owned()
        }
        Rule::TokenOverCeiling { field } => {
            format!("token field {field} over the {MAX_TOKENS_PER_FIELD} ceiling")
        }
    };
    format!("{file}: {place}: {rule}")
}
