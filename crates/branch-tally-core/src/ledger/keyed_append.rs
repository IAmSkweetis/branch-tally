//! Keyed appends that resolve the salt from the `.tally/` they write to (#67).
//!
//! The caller never holds a salt. It is resolved after the target `.tally/` is opened and from
//! that same [`TallyDir`], and only when at least one line will actually be appended. So a line is
//! never hashed under a salt that was not persisted next to it, a `.tally/` that appears after the
//! caller looked is not missed, and a run with nothing to write creates no salt.

use std::path::Path;

use super::redaction::SALT_FILE;
use super::{
    AppendOutcome, Clock, KeyedRedact, LEDGER_FILE, LedgerError, Plan, Salt, TallyDir, plan_append,
};
use crate::model::{Mode, SelectedRecord};

/// The default ledger, `.tally/ledger.jsonl` under `root`, appended with keyed redaction.
///
/// `main_root` is the main checkout when it differs from `root` (a linked worktree): its salt is
/// copied when the target has none, and it is never written. Writes and creates nothing when
/// `root` has no `.tally/` ([`AppendOutcome::tally_absent`]). `names` also hashes agent and skill
/// names.
///
/// # Errors
/// As [`super::append_default`], plus the salt errors of [`Salt::resolve`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `append_default` plus the salt inputs"
)]
pub fn append_keyed_default(
    root: &Path,
    main_root: Option<&Path>,
    names: bool,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    clock: &dyn Clock,
) -> Result<AppendOutcome, LedgerError> {
    append_keyed_default_with(
        root,
        main_root,
        names,
        records,
        recorded_for,
        mode,
        main,
        clock,
        false,
        &|| {},
    )
}

/// [`append_keyed_default`] that marks every line it writes `pending` when `pending` is set: the
/// branch being recorded is the repository's default branch (decision 0006).
///
/// # Errors
/// As [`append_keyed_default`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `append_keyed_default` plus the pending mark"
)]
pub fn append_keyed_default_pending(
    root: &Path,
    main_root: Option<&Path>,
    names: bool,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    clock: &dyn Clock,
    pending: bool,
) -> Result<AppendOutcome, LedgerError> {
    append_keyed_default_with(
        root,
        main_root,
        names,
        records,
        recorded_for,
        mode,
        main,
        clock,
        pending,
        &|| {},
    )
}

/// [`append_keyed_default`] with a seam that runs after the append is planned and before the salt
/// is resolved.
#[expect(
    clippy::too_many_arguments,
    reason = "`append_keyed_default` plus the seam"
)]
pub(super) fn append_keyed_default_with(
    root: &Path,
    main_root: Option<&Path>,
    names: bool,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    clock: &dyn Clock,
    pending: bool,
    between: &dyn Fn(),
) -> Result<AppendOutcome, LedgerError> {
    let Some(tally) = TallyDir::open(root)? else {
        return Ok(AppendOutcome {
            tally_absent: true,
            ..AppendOutcome::default()
        });
    };
    let main_tally = match main_root {
        Some(m) => TallyDir::open(m)?,
        None => None,
    };
    let existing = tally.read(LEDGER_FILE)?.unwrap_or_default();
    let Plan { buf, out } = keyed_plan(
        &existing,
        &tally,
        main_tally.as_ref(),
        names,
        &Input {
            records,
            recorded_for,
            mode,
            main,
            clock,
            pending,
        },
        between,
    )?;
    if out.appended > 0 {
        tally.append(LEDGER_FILE, buf.as_bytes())?;
    }
    Ok(out)
}

/// An explicit ledger file at `path` (`--ledger`), appended with keyed redaction under the salt
/// of `salt_home`, the main checkout's `.tally/` opened by the caller. Nothing is written, and no
/// salt is made, when there is nothing to append.
///
/// # Errors
/// As [`super::append`], plus the salt errors of [`Salt::resolve`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `append` plus the salt inputs"
)]
pub fn append_keyed_explicit(
    path: &Path,
    salt_home: &TallyDir,
    names: bool,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    clock: &dyn Clock,
) -> Result<AppendOutcome, LedgerError> {
    append_keyed_explicit_pending(
        path,
        salt_home,
        names,
        records,
        recorded_for,
        mode,
        main,
        clock,
        false,
    )
}

/// [`append_keyed_explicit`] with the pending mark of [`append_keyed_default_pending`].
///
/// # Errors
/// As [`append_keyed_explicit`].
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `append_keyed_explicit` plus the pending mark"
)]
pub fn append_keyed_explicit_pending(
    path: &Path,
    salt_home: &TallyDir,
    names: bool,
    records: &[SelectedRecord],
    recorded_for: &str,
    mode: Mode,
    main: Option<&Path>,
    clock: &dyn Clock,
    pending: bool,
) -> Result<AppendOutcome, LedgerError> {
    use std::io::Write;

    let write_err = |source| LedgerError::Write {
        path: path.to_owned(),
        source,
    };
    let existing = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(source) => {
            return Err(LedgerError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let Plan { buf, out } = keyed_plan(
        &existing,
        salt_home,
        None,
        names,
        &Input {
            records,
            recorded_for,
            mode,
            main,
            clock,
            pending,
        },
        &|| {},
    )?;
    if out.appended == 0 {
        return Ok(out);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(write_err)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(write_err)?;
    file.write_all(buf.as_bytes()).map_err(write_err)?;
    Ok(out)
}

/// What is being appended.
struct Input<'a> {
    records: &'a [SelectedRecord],
    recorded_for: &'a str,
    mode: Mode,
    main: Option<&'a Path>,
    clock: &'a dyn Clock,
    pending: bool,
}

impl Input<'_> {
    fn plan(&self, existing: &[u8], redactor: &KeyedRedact) -> Plan {
        plan_append(
            existing,
            self.records,
            self.recorded_for,
            self.mode,
            self.main,
            redactor,
            self.clock,
            self.pending,
        )
    }
}

/// Plans the append under the salt [`Salt::resolve`] settles on, creating that salt only when the
/// plan appends something.
///
/// Planning dedupes against held keys, which are hashed, so it needs a salt. The probe uses the
/// salt that already exists (the target's, else `main_tally`'s). When none exists no line hashed
/// under one can be held, so a throwaway salt gives the same answer: it appends exactly the
/// records not repeated within the call. Only a non-empty probe persists a salt, and the plan is
/// redone if the persisted salt is not the probe's, because a racer's salt won.
fn keyed_plan(
    existing: &[u8],
    tally: &TallyDir,
    main_tally: Option<&TallyDir>,
    names: bool,
    input: &Input<'_>,
    between: &dyn Fn(),
) -> Result<Plan, LedgerError> {
    let held = match Salt::load_settled(tally)? {
        Some(salt) => Some(salt),
        None => match main_tally {
            Some(m) => Salt::load_settled(m)?,
            None => None,
        },
    };
    let probe_salt = match held {
        Some(salt) => salt,
        None => Salt::generate().map_err(|source| LedgerError::Write {
            path: tally.file_path(SALT_FILE),
            source,
        })?,
    };
    let probe = input.plan(existing, &KeyedRedact::new(probe_salt.clone(), names));
    if probe.out.appended == 0 {
        return Ok(probe);
    }
    between();
    let salt = Salt::resolve(tally, main_tally)?;
    if salt == probe_salt {
        return Ok(probe);
    }
    Ok(input.plan(existing, &KeyedRedact::new(salt, names)))
}
