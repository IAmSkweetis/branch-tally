//! The `project` subcommand: read the ledger, call core's aggregator, print, optionally export
//! the chart. The binary only wires; every number comes from core.
//!
//! Errors are returned as the message to print; `main` writes it to stderr and exits 1.

use std::io::Write;
use std::path::Path;

use branch_tally_core::config::{Flags, Format};
use branch_tally_core::git::{Git, SystemGit};
use branch_tally_core::ledger::{self, Ledger, TallyDir};
use branch_tally_core::{project, render};

use crate::cli::ProjectArgs;
use crate::prices;
use crate::report::{
    Failure, Resolved, gate_ledger_conflicts, note_ledger_stats, resolve, stderr, stdout,
};

/// Runs the command. `Err` carries the one-line message for stderr.
pub(crate) fn run(args: &ProjectArgs) -> Result<(), Failure> {
    let Resolved { settings, cwd, .. } = resolve(
        &args.repo,
        args.config.as_deref(),
        &Flags {
            mode: None,
            work_branch: None,
            format: args.format_flag(),
            cost: None,
            redact: None,
            redact_names: None,
        },
        None,
    )?;
    let table = prices::resolve_table(&prices::Locations::for_repo(&cwd))?;
    let read = read_ledger(args.ledger.as_deref(), &cwd)?;
    gate_ledger_conflicts(read.stats().conflicts, args.allow_ledger_conflicts)?;
    prices::note_override(&table);
    note_ledger_stats(read.stats());
    if read.modes().len() > 1 {
        let names: Vec<&str> = read.modes().iter().map(|m| m.as_str()).collect();
        stderr(&format!(
            "note: ledger mixes modes ({}); the numbers combine them",
            names.join(", ")
        ));
    }
    let main = SystemGit.main_checkout(&cwd);
    let report = project::build(&read, Some(&main), settings.mode.value, &table);

    // Export first: a failed export must not leave a half-reported run behind a zero exit.
    if let Some(target) = &args.export_graph {
        write_atomic(target, render::project_svg(&report).as_bytes())?;
    }
    let text = match settings.format.value {
        Format::Json => render::project_json(&report),
        Format::Md => render::project_markdown(&report),
    };
    stdout(&format!("{}\n", text.trim_end_matches('\n')));
    Ok(())
}

/// The ledger to project. `--ledger` is read as given; the default is the checkout's guarded
/// `.tally/ledger.jsonl` (#94): a symlink is an error. As for `report --from-ledger`, no ledger
/// file is an error, not an empty report.
fn read_ledger(flag: Option<&Path>, cwd: &Path) -> Result<Ledger, String> {
    if let Some(path) = flag {
        if !path.exists() {
            return Err(format!("no ledger at {}", path.display()));
        }
        return ledger::read(path).map_err(|e| e.to_string());
    }
    let top = SystemGit.toplevel(cwd).map_err(|e| {
        format!(
            "cannot place the default ledger: {} is not inside a git checkout ({e}); \
             pass --ledger <path>",
            cwd.display()
        )
    })?;
    let present = match TallyDir::open(&top).map_err(|e| e.to_string())? {
        Some(tally) => tally
            .read("ledger.jsonl")
            .map_err(|e| e.to_string())?
            .is_some(),
        None => false,
    };
    if !present {
        return Err(format!(
            "no ledger at {}",
            top.join(".tally").join("ledger.jsonl").display()
        ));
    }
    ledger::read_default(&top).map_err(|e| e.to_string())
}

/// Writes `bytes` to `target` whole or not at all: a sibling temp file, then a rename. UTF-8 and
/// LF as given. The parent directory is not created.
fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write graph {}: {e}", target.display());
    let name = target
        .file_name()
        .ok_or_else(|| format!("cannot write graph {}: not a file path", target.display()))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = target.with_file_name(tmp_name);
    let written = std::fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, target)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}
