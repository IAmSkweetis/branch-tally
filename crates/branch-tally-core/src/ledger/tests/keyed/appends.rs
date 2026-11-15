//! `append_keyed_default` and `append_keyed_explicit`: the salt is resolved from the `.tally/`
//! that is written to, and only when a line will be appended.

use super::*;
use crate::ledger::keyed_append::append_keyed_default_with;

fn go(root: &Path, main_root: Option<&Path>, recs: &[SelectedRecord]) -> AppendOutcome {
    append_keyed_default(root, main_root, false, recs, "b", Mode::Claim, None, &CLOCK).unwrap()
}

fn salt_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(".tally").join(SALT_FILE)
}

fn persisted(dir: &tempfile::TempDir) -> Salt {
    Salt::parse(&std::fs::read(salt_path(dir)).unwrap()).unwrap()
}

#[test]
fn no_tally_dir_creates_no_salt_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = go(dir.path(), None, &[selected("a", "b", 1)]);
    assert!(out.tally_absent);
    assert_eq!(out.appended, 0);
    assert!(!dir.path().join(TALLY_DIR).exists());
}

#[test]
fn nothing_to_append_creates_no_salt() {
    let dir = opted_in();
    let out = go(dir.path(), None, &[]);
    assert_eq!(out.appended, 0);
    assert!(!salt_path(&dir).exists());
}

#[test]
fn lines_carry_the_fingerprint_of_the_salt_persisted_in_the_target() {
    let dir = opted_in();
    let out = go(
        dir.path(),
        None,
        &[selected("a", "b", 1), selected("c", "b", 2)],
    );
    assert_eq!(out.appended, 2);
    let salt = persisted(&dir);
    let l = read_default(dir.path()).unwrap();
    assert_eq!(l.entries().len(), 2);
    for e in l.entries() {
        assert_eq!(e.redaction, Some(salt.fingerprint()));
        assert!(is_hash(e.request_id.as_str()));
    }
    assert_eq!(l.entries()[0].request_id.as_str(), salt.hash("a"));
}

#[test]
fn a_second_identical_run_appends_nothing_and_keeps_the_salt() {
    let dir = opted_in();
    let recs = [selected("a", "b", 1), selected("c", "b", 2)];
    go(dir.path(), None, &recs);
    let salt = std::fs::read(salt_path(&dir)).unwrap();
    let again = go(dir.path(), None, &recs);
    assert_eq!((again.appended, again.skipped), (0, 2));
    assert_eq!(std::fs::read(salt_path(&dir)).unwrap(), salt);
    assert_eq!(read_default(dir.path()).unwrap().entries().len(), 2);
}

#[test]
fn a_target_without_a_salt_copies_the_main_checkouts_and_never_writes_main() {
    let (target, main) = (opted_in(), opted_in());
    let text = format!("{}\n", other_salt().to_hex());
    std::fs::write(salt_path(&main), &text).unwrap();
    let out = go(target.path(), Some(main.path()), &[selected("a", "b", 1)]);
    assert_eq!(out.appended, 1);
    assert_eq!(std::fs::read_to_string(salt_path(&target)).unwrap(), text);
    assert_eq!(std::fs::read_to_string(salt_path(&main)).unwrap(), text);
    let l = read_default(target.path()).unwrap();
    assert_eq!(l.entries()[0].redaction, Some(other_salt().fingerprint()));
}

#[test]
fn a_main_without_a_salt_is_left_without_one() {
    let (target, main) = (opted_in(), opted_in());
    go(target.path(), Some(main.path()), &[selected("a", "b", 1)]);
    assert!(salt_path(&target).exists());
    assert!(!salt_path(&main).exists());
}

#[test]
fn a_salt_that_appears_between_planning_and_resolving_is_the_one_used() {
    let dir = opted_in();
    let racer = format!("{}\n", other_salt().to_hex());
    let hook = || std::fs::write(salt_path(&dir), &racer).unwrap();
    let out = append_keyed_default_with(
        dir.path(),
        None,
        false,
        &[selected("a", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &CLOCK,
        &hook,
    )
    .unwrap();
    assert_eq!(out.appended, 1);
    let l = read_default(dir.path()).unwrap();
    assert_eq!(l.entries()[0].redaction, Some(other_salt().fingerprint()));
    assert_eq!(l.entries()[0].request_id.as_str(), other_salt().hash("a"));
}

#[test]
fn a_salt_still_being_written_by_a_racer_is_reread_once_complete() {
    // A racing creator has made `.tally/salt` but not yet written its bytes. The hook completes
    // it after the first malformed read, so the order is fixed without timing.
    let dir = opted_in();
    std::fs::write(salt_path(&dir), b"").unwrap();
    let racer = format!("{}\n", other_salt().to_hex());
    let hook = || std::fs::write(salt_path(&dir), &racer).unwrap();
    let tally = TallyDir::open(dir.path()).unwrap().unwrap();
    let got = Salt::load_settled_with(&tally, &hook).unwrap();
    assert_eq!(got, Some(other_salt()));
}

#[test]
fn a_damaged_salt_stops_the_append_and_writes_no_line() {
    let dir = opted_in();
    std::fs::write(salt_path(&dir), b"junk").unwrap();
    let err = append_keyed_default(
        dir.path(),
        None,
        false,
        &[selected("a", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &CLOCK,
    )
    .unwrap_err();
    assert!(matches!(err, LedgerError::MalformedSalt { .. }), "{err}");
    assert!(read_default(dir.path()).unwrap().is_empty());
}

#[test]
fn explicit_ledger_takes_its_salt_from_the_given_tally_dir() {
    let home = opted_in();
    let out_dir = tempfile::tempdir().unwrap();
    let p = out_dir.path().join("l.jsonl");
    let tally = TallyDir::open(home.path()).unwrap().unwrap();
    let recs = [selected("a", "b", 1)];
    // Nothing to append: no salt.
    append_keyed_explicit(&p, &tally, false, &[], "b", Mode::Claim, None, &CLOCK).unwrap();
    assert!(!salt_path(&home).exists());
    assert!(!p.exists());
    let out =
        append_keyed_explicit(&p, &tally, false, &recs, "b", Mode::Claim, None, &CLOCK).unwrap();
    assert_eq!(out.appended, 1);
    let salt = persisted(&home);
    assert_eq!(
        read(&p).unwrap().entries()[0].redaction,
        Some(salt.fingerprint())
    );
    let again =
        append_keyed_explicit(&p, &tally, false, &recs, "b", Mode::Claim, None, &CLOCK).unwrap();
    assert_eq!((again.appended, again.skipped), (0, 1));
}
