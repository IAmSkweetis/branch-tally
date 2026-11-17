//! Audit follow-ups to keyed redaction: salt beside its ledger, agent branch names, forged
//! stamps, the malformed-salt text and a ledger that changes under a rewrite.

use super::*;
use crate::ledger::redaction::{redact_ledger_default_with, redact_ledger_with};

fn tally_of(dir: &tempfile::TempDir) -> TallyDir {
    TallyDir::open(dir.path()).unwrap().unwrap()
}

fn salt_file(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(".tally").join(SALT_FILE)
}

// ---- 1. Salt::resolve ----

#[test]
fn resolve_uses_the_targets_own_salt_and_leaves_main_alone() {
    let (target, main) = (opted_in(), opted_in());
    let own = Salt::load_or_create(&tally_of(&target)).unwrap();
    std::fs::write(salt_file(&main), format!("{}\n", other_salt().to_hex())).unwrap();
    let got = Salt::resolve(&tally_of(&target), Some(&tally_of(&main))).unwrap();
    assert_eq!(got, own);
    assert_ne!(got, other_salt());
}

#[test]
fn resolve_copies_the_main_salt_into_a_target_without_one() {
    let (target, main) = (opted_in(), opted_in());
    let text = format!("{}\n", other_salt().to_hex());
    std::fs::write(salt_file(&main), &text).unwrap();
    let got = Salt::resolve(&tally_of(&target), Some(&tally_of(&main))).unwrap();
    assert_eq!(got, other_salt());
    assert_eq!(std::fs::read_to_string(salt_file(&target)).unwrap(), text);
    assert_eq!(std::fs::read_to_string(salt_file(&main)).unwrap(), text);
}

#[test]
fn resolve_creates_in_the_target_only_when_main_has_none() {
    let (target, main) = (opted_in(), opted_in());
    let got = Salt::resolve(&tally_of(&target), Some(&tally_of(&main))).unwrap();
    assert_eq!(
        Salt::parse(&std::fs::read(salt_file(&target)).unwrap()),
        Some(got)
    );
    assert!(!salt_file(&main).exists(), "main must never be written");
    let alone = opted_in();
    let made = Salt::resolve(&tally_of(&alone), None).unwrap();
    assert_eq!(Salt::load(&tally_of(&alone)).unwrap(), Some(made));
}

#[test]
fn resolve_with_a_damaged_main_salt_is_an_error_and_writes_nothing() {
    let (target, main) = (opted_in(), opted_in());
    std::fs::write(salt_file(&main), b"junk").unwrap();
    assert!(matches!(
        Salt::resolve(&tally_of(&target), Some(&tally_of(&main))),
        Err(LedgerError::MalformedSalt { .. })
    ));
    assert!(!salt_file(&target).exists());
}

#[test]
fn racing_resolvers_all_get_the_main_salt() {
    let (target, main) = (opted_in(), opted_in());
    std::fs::write(salt_file(&main), format!("{}\n", other_salt().to_hex())).unwrap();
    let (t, m) = (target.path().to_owned(), main.path().to_owned());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (t, m) = (t.clone(), m.clone());
            std::thread::spawn(move || {
                let (t, m) = (
                    TallyDir::open(&t).unwrap().unwrap(),
                    TallyDir::open(&m).unwrap().unwrap(),
                );
                Salt::resolve(&t, Some(&m)).unwrap()
            })
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), other_salt());
    }
}

// ---- 2. worktree-agent-<id> branch names ----

#[test]
fn an_agent_worktree_branch_is_hashed_and_other_names_are_not() {
    let r = KeyedRedact::new(test_salt(), false);
    let want = format!("worktree-agent-{}", test_salt().hash("a1b2c3"));
    assert_eq!(r.redact_branch("worktree-agent-a1b2c3"), want);
    // A requested name is always hashed, even when its id happens to look like a hash.
    let raw_hex = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        r.redact_branch(&format!("worktree-agent-{raw_hex}")),
        format!("worktree-agent-{}", test_salt().hash(raw_hex))
    );
    for plain in [
        "main",
        "feat/x",
        "worktree-agent-",
        "worktree-other-a1",
        "my-worktree-agent-a1",
    ] {
        assert_eq!(r.redact_branch(plain), plain);
    }
}

fn agent_branch_record() -> SelectedRecord {
    selected("r1", "worktree-agent-a1b2c3", 5)
}

#[test]
fn branch_and_recorded_for_are_hashed_in_an_entry_and_in_a_live_record() {
    let r = KeyedRedact::new(test_salt(), false);
    let want = format!("worktree-agent-{}", test_salt().hash("a1b2c3"));
    let e = LedgerEntry::from_selected(
        &agent_branch_record(),
        "worktree-agent-a1b2c3",
        Mode::Claim,
        None,
    );
    let after = r.redact(e);
    assert_eq!(
        (after.branch.as_str(), after.recorded_for.as_str()),
        (want.as_str(), want.as_str())
    );
    let live = r.redact_selected(agent_branch_record(), None);
    assert_eq!(live.attributed.branch, want);
}

#[test]
fn the_backfill_hashes_agent_branches_too() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let e = LedgerEntry::from_selected(
        &agent_branch_record(),
        "worktree-agent-a1b2c3",
        Mode::Claim,
        None,
    );
    write_lines(&p, &[serde_json::to_string(&e).unwrap()]);
    redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("a1b2c3"), "{text}");
}

// ---- 3. a forged stamp ----

fn forged_line() -> String {
    let mut e = full_entry();
    e.redaction = Some(test_salt().fingerprint());
    serde_json::to_string(&e).unwrap()
}

#[test]
fn a_stamp_over_raw_ids_is_not_believed_by_redact() {
    let mut e = full_entry();
    e.redaction = Some("deadbeefdeadbeef".to_owned());
    let after = KeyedRedact::new(test_salt(), false).redact(e);
    assert!(is_hash(after.request_id.as_str()));
    assert!(is_hash(after.session_id.as_str()));
    assert_eq!(after.redaction, Some(test_salt().fingerprint()));
}

#[test]
fn a_genuine_stamp_with_hashed_ids_is_left_alone() {
    let r = KeyedRedact::new(test_salt(), false);
    let once = r.redact(full_entry());
    assert_eq!(r.redact(once.clone()), once);
}

#[test]
fn the_backfill_rehashes_and_counts_a_forged_stamp_separately() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let genuine = redacted_line("g", &test_salt());
    write_lines(&p, &[forged_line(), genuine.clone()]);
    let out = redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    assert_eq!(
        (
            out.rewritten,
            out.restamped,
            out.already_redacted,
            out.unparseable
        ),
        (0, 1, 1, 0)
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("req-9"), "{text}");
    assert_eq!(text.lines().nth(1).unwrap(), genuine);
}

// ---- 4. the malformed-salt text ----

#[test]
fn the_malformed_salt_message_says_what_to_do_and_what_it_costs() {
    let dir = opted_in();
    std::fs::write(salt_file(&dir), b"").unwrap();
    let err = Salt::load(&tally_of(&dir)).unwrap_err().to_string();
    assert!(err.contains("salt"), "{err}");
    assert!(err.contains("empty or damaged"), "{err}");
    assert!(err.contains("delete"), "{err}");
    assert!(err.contains("different salt"), "{err}");
}

// ---- 5. a ledger that changes under the rewrite ----

fn unredacted_ledger(p: &Path) {
    write_lines(p, &[line("a", 1, None)]);
}

#[test]
fn a_ledger_appended_to_during_the_rewrite_is_not_replaced_and_leaves_no_temp() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    unredacted_ledger(&p);
    let appended = line("late", 2, None);
    let hook = || {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{appended}").unwrap();
    };
    let err = redact_ledger_with(&p, &KeyedRedact::new(test_salt(), false), &hook).unwrap_err();
    assert!(
        matches!(err, LedgerError::ChangedDuringRewrite { .. }),
        "{err}"
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("\"late\""), "the concurrent append survives");
    assert!(!text.contains("redaction"), "nothing was rewritten");
    assert_eq!(
        std::fs::read_dir(p.parent().unwrap()).unwrap().count(),
        1,
        "no temp left"
    );
}

#[test]
fn the_default_ledger_is_guarded_the_same_way() {
    let dir = opted_in();
    let p = dir.path().join(".tally").join("ledger.jsonl");
    unredacted_ledger(&p);
    let appended = line("late", 2, None);
    let hook = || {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{appended}").unwrap();
    };
    let err = redact_ledger_default_with(dir.path(), &KeyedRedact::new(test_salt(), false), &hook)
        .unwrap_err();
    assert!(
        matches!(err, LedgerError::ChangedDuringRewrite { .. }),
        "{err}"
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("\"late\"") && !text.contains("redaction"));
    assert_eq!(
        std::fs::read_dir(dir.path().join(".tally"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn an_unchanged_ledger_is_still_replaced() {
    let dir = opted_in();
    let p = dir.path().join(".tally").join("ledger.jsonl");
    unredacted_ledger(&p);
    let out = redact_ledger_default_with(dir.path(), &KeyedRedact::new(test_salt(), false), &|| {})
        .unwrap();
    assert_eq!(out.rewritten, 1);
}

#[test]
fn replace_if_unchanged_checks_the_expected_bytes() {
    let dir = opted_in();
    let tally = tally_of(&dir);
    tally.append("f", b"old\n").unwrap();
    assert!(matches!(
        tally.replace_if_unchanged("f", b"new\n", b"different\n"),
        Err(LedgerError::ChangedDuringRewrite { .. })
    ));
    assert_eq!(tally.read("f").unwrap().unwrap(), b"old\n");
    tally.replace_if_unchanged("f", b"new\n", b"old\n").unwrap();
    assert_eq!(tally.read("f").unwrap().unwrap(), b"new\n");
    assert_eq!(std::fs::read_dir(tally.path()).unwrap().count(), 1);
}

// ---- re-audit: stamps and agent branches ----

fn stamped_with_raw_agent_branch() -> LedgerEntry {
    // What an earlier build wrote: hashed ids, a stamp, and a raw agent branch.
    let mut e = KeyedRedact::new(test_salt(), false).redact(full_entry());
    e.branch = "worktree-agent-a17hexid".to_owned();
    e.recorded_for = "worktree-agent-a17hexid".to_owned();
    e
}

#[test]
fn a_stamped_line_with_a_raw_agent_branch_is_repaired_without_rehashing_ids() {
    let r = KeyedRedact::new(test_salt(), false);
    let before = stamped_with_raw_agent_branch();
    let after = r.redact(before.clone());
    let want = format!("worktree-agent-{}", test_salt().hash("a17hexid"));
    assert_eq!(
        (after.branch.as_str(), after.recorded_for.as_str()),
        (want.as_str(), want.as_str())
    );
    assert_eq!(
        after.request_id, before.request_id,
        "ids are already hashed"
    );
    assert_eq!(after.session_id, before.session_id);
    assert_eq!(after.redaction, Some(test_salt().fingerprint()));
    // Now it is trustworthy, and a further pass changes nothing.
    assert_eq!(r.redact(after.clone()), after);
}

fn raw_branch_line_under(salt: &Salt) -> LedgerEntry {
    let mut e = KeyedRedact::new(salt.clone(), false).redact(full_entry());
    e.branch = "worktree-agent-a17hexid".to_owned();
    e.recorded_for = "worktree-agent-a17hexid".to_owned();
    e
}

#[test]
fn a_foreign_stamped_line_with_a_raw_agent_branch_is_left_unchanged_by_redact() {
    let before = raw_branch_line_under(&other_salt());
    let after = KeyedRedact::new(test_salt(), false).redact(before.clone());
    assert_eq!(after, before);
}

#[test]
fn the_backfill_leaves_a_foreign_stamped_raw_branch_line_byte_identical_and_counts_it() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let foreign = serde_json::to_string(&raw_branch_line_under(&other_salt())).unwrap();
    let own = serde_json::to_string(&raw_branch_line_under(&test_salt())).unwrap();
    write_lines(&p, &[foreign.clone(), own]);
    let out = redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    assert_eq!(
        (
            out.restamped,
            out.foreign_salt_unrepaired,
            out.already_redacted
        ),
        (1, 1, 0)
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert_eq!(text.lines().next().unwrap(), foreign);
    assert!(
        text.lines()
            .nth(1)
            .unwrap()
            .contains(&test_salt().hash("a17hexid")),
        "the line under the current salt is repaired"
    );
}

#[test]
fn a_properly_stamped_agent_branch_is_never_hashed_twice() {
    let r = KeyedRedact::new(test_salt(), false);
    let mut e = full_entry();
    e.branch = "worktree-agent-a1b2c3".to_owned();
    e.recorded_for = e.branch.clone();
    let once = r.redact(e);
    assert_eq!(r.redact(once.clone()), once);
}

#[test]
fn the_backfill_restamps_a_stamped_line_with_a_raw_agent_branch() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    write_lines(
        &p,
        &[serde_json::to_string(&stamped_with_raw_agent_branch()).unwrap()],
    );
    let out = redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    assert_eq!(
        (out.rewritten, out.restamped, out.already_redacted),
        (0, 1, 0)
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("a17hexid"), "{text}");
    let again = redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    assert_eq!((again.restamped, again.already_redacted), (0, 1));
}

// ---- 6. an over-long line swapped in for a literal marker line ----

/// A ledger of a literal NUL line (the byte the bounded reader uses for an over-long line) and
/// an unredacted entry, so a rewrite is wanted.
fn nul_line_ledger(p: &Path) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, format!("\0\n{}\n", line("a", 1, None))).unwrap();
}

/// What the concurrent writer leaves: the NUL line replaced by an over-long line.
fn overlong_swap(p: &Path) {
    let mut text = "x".repeat(MAX_LINE_BYTES + 1);
    text.push('\n');
    text.push_str(&line("a", 1, None));
    text.push('\n');
    std::fs::write(p, text).unwrap();
}

fn assert_concurrent_overlong_intact(p: &Path, err: &LedgerError) {
    assert!(
        matches!(err, LedgerError::ChangedDuringRewrite { .. }),
        "{err}"
    );
    let text = std::fs::read_to_string(p).unwrap();
    assert!(text.starts_with("xxxx"), "the concurrent content survives");
    assert!(!text.contains("redaction"), "nothing was rewritten");
    assert_eq!(
        std::fs::read_dir(p.parent().unwrap()).unwrap().count(),
        1,
        "no temp left"
    );
}

#[test]
fn an_overlong_line_swapped_for_a_nul_line_counts_as_changed() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    nul_line_ledger(&p);
    let hook = || overlong_swap(&p);
    let err = redact_ledger_with(&p, &KeyedRedact::new(test_salt(), false), &hook).unwrap_err();
    assert_concurrent_overlong_intact(&p, &err);
}

#[test]
fn the_default_ledger_treats_a_swapped_in_overlong_line_as_changed_too() {
    let dir = opted_in();
    let p = dir.path().join(".tally").join("ledger.jsonl");
    nul_line_ledger(&p);
    let hook = || overlong_swap(&p);
    let err = redact_ledger_default_with(dir.path(), &KeyedRedact::new(test_salt(), false), &hook)
        .unwrap_err();
    assert_concurrent_overlong_intact(&p, &err);
}
