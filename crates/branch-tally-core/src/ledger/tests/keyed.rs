//! Keyed redaction (decision 0005, #67): the salt, the redactor, read stats and `ledger redact`.

use super::*;

fn test_salt() -> Salt {
    let mut b = [0_u8; 32];
    for (i, x) in b.iter_mut().enumerate() {
        *x = u8::try_from(i).unwrap();
    }
    Salt::from_bytes(b)
}

fn other_salt() -> Salt {
    Salt::from_bytes([7; 32])
}

fn is_hash(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[test]
fn hash_is_hmac_sha256_truncated_to_32_lowercase_hex() {
    assert_eq!(
        test_salt().hash("req_1"),
        "9dccd1fb1f2cca794523c920547dca12"
    );
    assert_ne!(other_salt().hash("req_1"), test_salt().hash("req_1"));
}

#[test]
fn fingerprint_is_the_first_16_hex_of_sha256_over_the_salt() {
    assert_eq!(test_salt().fingerprint(), "630dcd2966c43366");
}

#[test]
fn salt_hex_round_trips_and_rejects_anything_else() {
    let hex = test_salt().to_hex();
    assert_eq!(
        hex,
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
    );
    assert_eq!(Salt::parse(hex.as_bytes()), Some(test_salt()));
    assert_eq!(
        Salt::parse(format!("{hex}\n").as_bytes()),
        Some(test_salt())
    );
    assert_eq!(Salt::parse(hex.to_uppercase().as_bytes()), None);
    assert_eq!(Salt::parse(&hex.as_bytes()[..62]), None);
    assert_eq!(Salt::parse(format!("{hex}00").as_bytes()), None);
    assert_eq!(Salt::parse(b""), None);
}

#[test]
fn salt_debug_never_prints_the_salt() {
    let shown = format!("{:?}", test_salt());
    assert!(!shown.contains(&test_salt().to_hex()), "{shown}");
}

fn full_entry() -> LedgerEntry {
    let mut e = LedgerEntry::from_selected(&selected("req-9", "b", 5), "b", Mode::Claim, None);
    e.recorded_at = Some("2026-02-02T02:02:02Z".to_owned());
    e
}

#[test]
fn redaction_hashes_ids_and_leaves_everything_else() {
    let before = full_entry();
    let after = KeyedRedact::new(test_salt(), false).redact(before.clone());
    let s = test_salt();
    assert_eq!(after.request_id.as_str(), s.hash("req-9"));
    assert_eq!(after.session_id.as_str(), s.hash("sess-1"));
    assert_eq!(after.agent_id.as_ref().unwrap().as_str(), s.hash("agent-1"));
    let tags = after.subagent.as_ref().unwrap();
    assert_eq!(tags.agent_id.as_str(), s.hash("agent-1"));
    assert_eq!(tags.subagent, "ag", "a name stays without the names opt-in");
    assert_eq!(
        (after.skill.as_deref(), after.agent.as_deref()),
        (Some("sk"), Some("ag"))
    );
    assert_eq!(after.redaction.as_deref(), Some("630dcd2966c43366"));
    let mut expected = before;
    expected.request_id = after.request_id.clone();
    expected.session_id = after.session_id.clone();
    expected.agent_id = after.agent_id.clone();
    expected.subagent = after.subagent.clone();
    expected.redaction = after.redaction.clone();
    assert_eq!(
        after, expected,
        "ts, recorded_at, model, branch, usage unchanged"
    );
}

#[test]
fn names_are_hashed_only_under_the_names_option() {
    let after = KeyedRedact::new(test_salt(), true).redact(full_entry());
    let s = test_salt();
    assert_eq!(after.skill.as_deref(), Some(s.hash("sk").as_str()));
    assert_eq!(after.agent.as_deref(), Some(s.hash("ag").as_str()));
    assert_eq!(after.subagent.unwrap().subagent, s.hash("ag"));
}

#[test]
fn a_subagent_tag_that_is_the_raw_agent_id_is_hashed_even_without_names() {
    let mut e = full_entry();
    e.subagent.as_mut().unwrap().subagent = "agent-1".to_owned();
    let after = KeyedRedact::new(test_salt(), false).redact(e);
    let tags = after.subagent.unwrap();
    assert_eq!(tags.subagent, test_salt().hash("agent-1"));
    assert_eq!(tags.subagent, tags.agent_id.as_str());
}

#[test]
fn redaction_is_idempotent_and_never_hashes_a_redacted_line_again() {
    let r = KeyedRedact::new(test_salt(), true);
    let once = r.redact(full_entry());
    assert_eq!(r.redact(once.clone()), once);
    // Even a different salt leaves an already-redacted line alone.
    assert_eq!(
        KeyedRedact::new(other_salt(), true).redact(once.clone()),
        once
    );
}

#[test]
fn a_redacted_line_has_only_hashes_for_ids() {
    let after = KeyedRedact::new(test_salt(), false).redact(full_entry());
    assert!(is_hash(after.request_id.as_str()));
    assert!(is_hash(after.session_id.as_str()));
    assert!(is_hash(after.agent_id.as_ref().unwrap().as_str()));
}

#[test]
fn redact_selected_matches_the_ledger_entry_round_trip_on_hashed_fields() {
    let r = KeyedRedact::new(test_salt(), false);
    let rec = selected("req-9", "b", 5);
    let live = r.redact_selected(rec.clone(), None);
    let via_ledger = r
        .redact(LedgerEntry::from_selected(&rec, "b", Mode::Claim, None))
        .to_selected(None);
    let (a, b) = (&live.attributed, &via_ledger.attributed);
    assert_eq!(a.record.request_id, b.record.request_id);
    assert_eq!(a.record.session_id, b.record.session_id);
    assert_eq!(a.record.agent_id, b.record.agent_id);
    assert_eq!(a.subagent, b.subagent);
    assert_eq!(a.record.usage, b.record.usage);
}

#[test]
fn salt_is_created_once_and_read_back() {
    let dir = opted_in();
    let tally = TallyDir::open(dir.path()).unwrap().unwrap();
    assert_eq!(Salt::load(&tally).unwrap(), None);
    let first = Salt::load_or_create(&tally).unwrap();
    let on_disk = std::fs::read(dir.path().join(".tally").join(SALT_FILE)).unwrap();
    assert_eq!(Salt::parse(&on_disk), Some(first.clone()));
    assert_eq!(Salt::load_or_create(&tally).unwrap(), first);
    assert_eq!(Salt::load(&tally).unwrap(), Some(first));
}

#[test]
fn a_malformed_salt_is_an_error_and_is_never_replaced() {
    for bad in [&b"not a salt"[..], b"", b"ABCDEF"] {
        let dir = opted_in();
        let file = dir.path().join(".tally").join(SALT_FILE);
        std::fs::write(&file, bad).unwrap();
        let tally = TallyDir::open(dir.path()).unwrap().unwrap();
        for result in [Salt::load(&tally), Salt::load_or_create(&tally).map(Some)] {
            assert!(
                matches!(result, Err(LedgerError::MalformedSalt { .. })),
                "{result:?}"
            );
        }
        assert_eq!(std::fs::read(&file).unwrap(), bad);
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_salt_is_refused_and_the_target_untouched() {
    let dir = opted_in();
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"precious").unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join(".tally").join(SALT_FILE)).unwrap();
    let tally = TallyDir::open(dir.path()).unwrap().unwrap();
    assert!(matches!(
        Salt::load_or_create(&tally),
        Err(LedgerError::Refused { .. })
    ));
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
}

#[test]
fn racing_creators_agree_on_one_salt() {
    let dir = opted_in();
    let root = dir.path().to_owned();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || {
                let tally = TallyDir::open(&root).unwrap().unwrap();
                Salt::load_or_create(&tally).unwrap()
            })
        })
        .collect();
    let salts: Vec<Salt> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(salts.iter().all(|s| *s == salts[0]), "{salts:?}");
    let on_disk = std::fs::read(root.join(".tally").join(SALT_FILE)).unwrap();
    assert_eq!(Salt::parse(&on_disk), Some(salts[0].clone()));
}

fn redacted_line(id: &str, salt: &Salt) -> String {
    let e = KeyedRedact::new(salt.clone(), false).redact(LedgerEntry::from_selected(
        &selected(id, "b", 1),
        "b",
        Mode::Claim,
        None,
    ));
    serde_json::to_string(&e).unwrap()
}

#[test]
fn read_stats_count_unredacted_redacted_and_distinct_fingerprints() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    write_lines(
        &p,
        &[
            line("a", 1, None),
            line("b", 1, None),
            redacted_line("c", &test_salt()),
            redacted_line("d", &other_salt()),
            redacted_line("e", &other_salt()),
            "garbage".to_owned(),
        ],
    );
    let stats = read(&p).unwrap().stats().clone();
    assert_eq!(stats.unredacted_lines, 2);
    assert_eq!(stats.redacted_lines, 3);
    let mut want = vec![other_salt().fingerprint(), test_salt().fingerprint()];
    want.sort();
    assert_eq!(stats.redaction_fingerprints, want);
    assert!(stats.is_mixed());
}

#[test]
fn a_uniform_ledger_is_not_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    write_lines(
        &p,
        &[
            redacted_line("a", &test_salt()),
            redacted_line("b", &test_salt()),
        ],
    );
    assert!(!read(&p).unwrap().stats().is_mixed());
    write_lines(&p, &[line("a", 1, None)]);
    assert!(!read(&p).unwrap().stats().is_mixed());
}

#[test]
fn redact_ledger_rewrites_unredacted_lines_and_leaves_the_rest_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let already = redacted_line("c", &other_salt());
    write_lines(
        &p,
        &[
            line("a", 1, Some("2026-01-01T00:00:00Z")),
            "garbage {".to_owned(),
            already.clone(),
        ],
    );
    let r = KeyedRedact::new(test_salt(), false);
    let out = redact_ledger(&p, &r).unwrap();
    assert_eq!(
        (
            out.lines,
            out.rewritten,
            out.already_redacted,
            out.unparseable
        ),
        (3, 1, 1, 1)
    );
    let text = std::fs::read_to_string(&p).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "garbage {");
    assert_eq!(lines[2], already);
    let l: LedgerEntry = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(l.request_id.as_str(), test_salt().hash("a"));
    assert_eq!(l.recorded_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(l.redaction, Some(test_salt().fingerprint()));
    assert!(!lines[0].contains("\"a\""), "raw id gone");
    assert_eq!(
        std::fs::read_dir(p.parent().unwrap()).unwrap().count(),
        1,
        "no temp file left"
    );
    // A second run has nothing to do.
    let again = redact_ledger(&p, &r).unwrap();
    assert_eq!(
        (again.rewritten, again.already_redacted, again.unparseable),
        (0, 2, 1)
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
}

#[test]
fn redact_ledger_on_a_missing_file_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let out = redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    assert_eq!(out.lines, 0);
    assert!(!p.exists());
}

#[test]
fn redact_ledger_default_goes_through_the_guard() {
    let r = KeyedRedact::new(test_salt(), false);
    let absent = tempfile::tempdir().unwrap();
    assert!(
        redact_ledger_default(absent.path(), &r)
            .unwrap()
            .tally_absent
    );
    assert!(!absent.path().join(TALLY_DIR).exists());

    let dir = opted_in();
    append_default(
        dir.path(),
        &[selected("a", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let out = redact_ledger_default(dir.path(), &r).unwrap();
    assert_eq!((out.rewritten, out.tally_absent), (1, false));
    let l = read_default(dir.path()).unwrap();
    assert_eq!(l.entries()[0].request_id.as_str(), test_salt().hash("a"));
}

#[cfg(unix)]
#[test]
fn redact_ledger_default_refuses_a_symlinked_ledger() {
    let dir = opted_in();
    let victim = dir.path().join("victim");
    let content = format!("{}\n", line("a", 1, None));
    std::fs::write(&victim, &content).unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join(".tally").join("ledger.jsonl")).unwrap();
    let r = KeyedRedact::new(test_salt(), false);
    assert!(matches!(
        redact_ledger_default(dir.path(), &r),
        Err(LedgerError::Refused { .. })
    ));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), content);
}

#[test]
fn append_with_a_keyed_redactor_dedupes_on_the_hashed_key() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let r = KeyedRedact::new(test_salt(), false);
    let recs = [selected("a", "b", 1), selected("c", "b", 2)];
    let first = append(&p, &recs, "b", Mode::Claim, None, &r, &CLOCK).unwrap();
    let second = append(&p, &recs, "b", Mode::Claim, None, &r, &CLOCK).unwrap();
    assert_eq!((first.appended, second.appended, second.skipped), (2, 0, 2));
}

// ---- worktree names (decision 0005, amended) ----

const MAIN: &str = "/repo";

fn worktree_record(id: &str, name: &str) -> SelectedRecord {
    let mut rec = selected(id, "b", 5);
    let slug = format!(
        "{}--claude-worktrees-{name}",
        crate::paths::slug(Path::new(MAIN))
    );
    rec.attributed.record.origin = Some(Origin::worktree(slug));
    rec.attributed.subagent = Some(SubagentTags {
        workspace: Workspace::worktree(name),
        subagent: "ag".to_owned(),
        agent_id: AgentId::new("agent-1"),
    });
    rec
}

fn worktree_entry(name: &str) -> LedgerEntry {
    LedgerEntry::from_selected(
        &worktree_record("r1", name),
        "b",
        Mode::Claim,
        Some(Path::new(MAIN)),
    )
}

#[test]
fn worktree_names_are_hashed_in_tags_and_origin_regardless_of_names() {
    let name = "agent-a1b2c3";
    for names in [false, true] {
        let after = KeyedRedact::new(test_salt(), names).redact(worktree_entry(name));
        let hashed = test_salt().hash(&format!("claude-worktrees-{name}"));
        assert_eq!(
            after.origin,
            Some(StoredOrigin::Worktree(Some(hashed))),
            "{names}"
        );
        assert_eq!(
            after.subagent.unwrap().workspace,
            Workspace::worktree(test_salt().hash(name))
        );
        let text = serde_json::to_string(
            &KeyedRedact::new(test_salt(), names).redact(worktree_entry(name)),
        )
        .unwrap();
        assert!(!text.contains("a1b2c3"), "{text}");
    }
}

#[test]
fn main_and_unnamed_worktree_origins_stay_as_they_are() {
    let r = KeyedRedact::new(test_salt(), false);
    assert_eq!(r.redact_origin(&StoredOrigin::Main), StoredOrigin::Main);
    assert_eq!(
        r.redact_origin(&StoredOrigin::Worktree(None)),
        StoredOrigin::Worktree(None)
    );
    let mut e = worktree_entry("agent-x");
    e.origin = Some(StoredOrigin::Worktree(None));
    assert_eq!(r.redact(e).origin, Some(StoredOrigin::Worktree(None)));
}

#[test]
fn a_live_record_hashed_with_the_main_checkout_matches_the_ledger_entry_origin() {
    let r = KeyedRedact::new(test_salt(), false);
    let main = Some(Path::new(MAIN));
    let rec = worktree_record("r1", "agent-a1b2c3");
    let live = r.redact_selected(rec.clone(), main);
    let stored = r
        .redact(LedgerEntry::from_selected(&rec, "b", Mode::Claim, main))
        .to_selected(main);
    assert_eq!(
        live.attributed.record.origin,
        stored.attributed.record.origin
    );
    assert_eq!(live.attributed.subagent, stored.attributed.subagent);
    assert!(live.attributed.record.origin.is_some());
    // A worktree outside the main checkout stays unknown, as it reads back from the ledger.
    let mut outside = rec;
    outside.attributed.record.origin = Some(Origin::worktree("elsewhere--agent-q"));
    assert_eq!(
        r.redact_selected(outside, main).attributed.record.origin,
        None
    );
}

#[test]
fn backfill_hashes_worktree_names() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let e = worktree_entry("agent-a1b2c3");
    write_lines(&p, &[serde_json::to_string(&e).unwrap()]);
    redact_ledger(&p, &KeyedRedact::new(test_salt(), false)).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(!text.contains("a1b2c3"), "{text}");
    assert!(text.contains(&test_salt().hash("agent-a1b2c3")));
}
