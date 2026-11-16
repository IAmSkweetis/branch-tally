use super::*;
use crate::ledger::{Resolved, SCHEMA_VERSION, parse_ledger};
use crate::model::Selection;

const NOW: &str = "2026-02-01T00:00:00Z";

fn line(rid: &str, recorded_for: &str, pending: bool, ts: Option<&str>) -> String {
    let mut v = serde_json::json!({
        "v": SCHEMA_VERSION,
        "request_id": rid,
        "harness": "claude-code",
        "session_id": "s1",
        "model": "claude-x",
        "recorded_for": recorded_for,
        "mode": "claim",
        "branch": recorded_for,
        "basis": "stamped",
        "kept_by": "exact",
        "usage": {"input": 1, "cache_write_5m": 2, "cache_write_1h": 3, "cache_read": 4, "output": 5},
        "recorded_at": "2026-01-01T00:00:00Z",
    });
    if pending {
        v["pending"] = true.into();
    }
    if let Some(ts) = ts {
        v["ts"] = ts.into();
    }
    v.to_string()
}

fn resolution_line(rid: &str, to: &str, from: &str) -> String {
    let mut v: serde_json::Value =
        serde_json::from_str(&line(rid, to, false, Some("2026-01-01T00:00:00Z"))).unwrap();
    v["resolved"] = serde_json::json!({ "from": from });
    v.to_string()
}

fn ledger(lines: &[String]) -> Ledger {
    parse_ledger(lines.join("\n").as_bytes())
}

fn forward(branch: &str) -> PendingEvidence {
    PendingEvidence {
        timeline: Some(TimelineEvidence {
            branch: branch.to_owned(),
            basis: BranchBasis::TimelineForward,
        }),
        capture: None,
    }
}

fn evidence(rid: &str, e: PendingEvidence) -> EvidenceMap {
    EvidenceMap::from([(RequestId::new(rid), e)])
}

fn pending_main(rid: &str) -> String {
    line(rid, "main", true, Some("2026-01-01T00:00:00Z"))
}

#[test]
fn a_pending_line_resolves_via_timeline_forward_as_a_copy() {
    let l = ledger(&[pending_main("r1")]);
    let r = resolve_pending(&l, "feat/x", &evidence("r1", forward("feat/x")), NOW);
    assert_eq!(r.stats.resolved, 1);
    assert_eq!(r.entries.len(), 1);
    let e = &r.entries[0];
    let p = &l.entries()[0];
    assert_eq!(e.recorded_for, "feat/x");
    assert_eq!(e.branch, "feat/x");
    assert_eq!(e.basis, BranchBasis::TimelineForward);
    assert_eq!(e.kept_by, Selection::Exact);
    assert!(!e.pending);
    assert_eq!(
        e.resolved,
        Some(Resolved {
            from: "main".to_owned()
        })
    );
    assert_eq!(e.recorded_at.as_deref(), Some(NOW));
    // Copied, not recomputed.
    assert_eq!(e.usage, p.usage);
    assert_eq!(e.request_id, p.request_id);
    assert_eq!(e.ts, p.ts);
    assert_eq!(e.session_id, p.session_id);
    assert_eq!(e.model, p.model);
    assert_eq!(e.mode, p.mode);
}

#[test]
fn a_subagent_line_resolves_with_the_dispatcher_basis() {
    let l = ledger(&[pending_main("r1")]);
    let ev = PendingEvidence {
        timeline: Some(TimelineEvidence {
            branch: "feat/x".to_owned(),
            basis: BranchBasis::DispatcherRolledUp,
        }),
        capture: None,
    };
    let r = resolve_pending(&l, "feat/x", &evidence("r1", ev), NOW);
    assert_eq!(r.entries[0].basis, BranchBasis::DispatcherRolledUp);
}

#[test]
fn a_pending_line_resolves_via_a_capture_alone() {
    let l = ledger(&[pending_main("r1")]);
    let ev = PendingEvidence {
        timeline: None,
        capture: Some("feat/x".to_owned()),
    };
    let r = resolve_pending(&l, "feat/x", &evidence("r1", ev), NOW);
    assert_eq!(r.stats.resolved, 1);
    assert_eq!(r.entries[0].basis, BranchBasis::Captured);
}

#[test]
fn agreeing_timeline_and_capture_resolve_with_the_timeline_basis() {
    let l = ledger(&[pending_main("r1")]);
    let mut ev = forward("feat/x");
    ev.capture = Some("feat/x".to_owned());
    let r = resolve_pending(&l, "feat/x", &evidence("r1", ev), NOW);
    assert_eq!(r.stats.resolved, 1);
    assert_eq!(r.entries[0].basis, BranchBasis::TimelineForward);
}

#[test]
fn a_request_another_branch_already_claims_is_covered_and_appends_nothing() {
    let l = ledger(&[
        pending_main("r1"),
        line("r1", "feat/x", false, Some("2026-01-01T00:00:00Z")),
    ]);
    let r = resolve_pending(&l, "feat/x", &evidence("r1", forward("feat/x")), NOW);
    assert_eq!(r.entries, []);
    assert_eq!(r.stats.covered, 1);
    assert_eq!(r.stats.resolved, 0);
}

#[test]
fn an_existing_resolution_to_the_target_is_covered() {
    let l = ledger(&[pending_main("r1"), resolution_line("r1", "feat/x", "main")]);
    let r = resolve_pending(&l, "feat/x", &evidence("r1", forward("feat/x")), NOW);
    assert_eq!(r.entries, []);
    assert_eq!(r.stats.covered, 1);
}

#[test]
fn a_request_already_resolved_to_a_different_branch_stays_pending_and_counted() {
    let l = ledger(&[
        pending_main("r1"),
        resolution_line("r1", "feat/other", "main"),
    ]);
    let r = resolve_pending(&l, "feat/x", &evidence("r1", forward("feat/x")), NOW);
    assert_eq!(r.entries, []);
    assert_eq!(r.stats.resolved_elsewhere, 1);
}

#[test]
fn an_ambiguous_timeline_stays_pending_and_is_counted() {
    let l = ledger(&[pending_main("r1")]);
    let ev = PendingEvidence {
        timeline: Some(TimelineEvidence {
            branch: "main".to_owned(),
            basis: BranchBasis::TimelineAmbiguous,
        }),
        capture: Some("feat/x".to_owned()),
    };
    let r = resolve_pending(&l, "feat/x", &evidence("r1", ev), NOW);
    assert_eq!(r.entries, []);
    assert_eq!(r.stats.ambiguous, 1);
    assert_eq!(r.stats.uncertain(), 1);
}

#[test]
fn transcripts_and_captures_that_disagree_stay_pending_and_are_counted() {
    let l = ledger(&[pending_main("r1")]);
    let mut ev = forward("feat/x");
    ev.capture = Some("feat/y".to_owned());
    let r = resolve_pending(&l, "feat/x", &evidence("r1", ev), NOW);
    assert_eq!(r.entries, []);
    assert_eq!(r.stats.disagreement, 1);
}

#[test]
fn each_stay_pending_reason_is_counted_under_its_own_field() {
    let basis = |b| PendingEvidence {
        timeline: Some(TimelineEvidence {
            branch: "main".to_owned(),
            basis: b,
        }),
        capture: None,
    };
    let l = ledger(&[
        pending_main("carried"),
        pending_main("stamped"),
        pending_main("unlinked"),
        pending_main("noevidence"),
        line("nots", "main", true, None),
    ]);
    let mut ev = EvidenceMap::new();
    ev.insert(
        RequestId::new("carried"),
        basis(BranchBasis::TimelineCarried),
    );
    ev.insert(RequestId::new("stamped"), basis(BranchBasis::Stamped));
    ev.insert(
        RequestId::new("unlinked"),
        basis(BranchBasis::SubagentUnlinked),
    );
    ev.insert(RequestId::new("nots"), forward("feat/x"));
    let r = resolve_pending(&l, "feat/x", &ev, NOW);
    assert_eq!(r.entries, []);
    let s = r.stats;
    assert_eq!(
        (
            s.carried_back,
            s.no_work_branch,
            s.subagent_unlinked,
            s.no_evidence,
            s.no_timestamp
        ),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(s.uncertain(), 5);
}

#[test]
fn a_line_whose_branch_was_cut_for_another_target_is_left_alone() {
    let l = ledger(&[pending_main("r1")]);
    let r = resolve_pending(&l, "feat/x", &evidence("r1", forward("feat/y")), NOW);
    assert_eq!(r.entries, []);
    assert_eq!(r.stats.other_branch, 1);
    assert_eq!(r.stats.uncertain(), 0);
}

#[test]
fn a_resolved_ledger_has_no_unresolved_pending_and_resolving_again_appends_nothing() {
    let first = ledger(&[pending_main("r1")]);
    let r = resolve_pending(&first, "feat/x", &evidence("r1", forward("feat/x")), NOW);
    let mut lines = vec![pending_main("r1")];
    lines.extend(r.entries.iter().map(|e| serde_json::to_string(e).unwrap()));
    let after = ledger(&lines);
    assert_eq!(after.unresolved_pending(), Pending::default());
    let again = resolve_pending(&after, "feat/x", &evidence("r1", forward("feat/x")), NOW);
    assert_eq!(again.entries, []);
}

#[test]
fn unresolved_pending_counts_lines_calls_and_tokens_outside_covered_and_resolved() {
    let l = ledger(&[
        pending_main("open1"),
        pending_main("open2"),
        pending_main("claimed"),
        line("claimed", "feat/x", false, None),
        pending_main("resolved"),
        resolution_line("resolved", "feat/y", "main"),
        line("plain", "main", false, None),
    ]);
    // Each line carries 1+2+3+4+5 = 15 tokens.
    assert_eq!(
        l.unresolved_pending(),
        Pending {
            lines: 2,
            calls: 2,
            tokens: 30
        }
    );
}

#[test]
fn unresolved_pending_counts_a_request_once_even_when_two_default_branches_hold_it() {
    // A repository whose default branch was renamed: the same request pending under both names.
    let l = ledger(&[
        line("r1", "main", true, None),
        line("r1", "master", true, None),
    ]);
    assert_eq!(l.unresolved_pending().lines, 1);
}

mod append {
    use super::*;
    use crate::ledger::{Clock, append_entries, append_entries_default, read, read_default};

    struct At;
    impl Clock for At {
        fn now(&self) -> String {
            "2026-03-01T00:00:00Z".to_owned()
        }
    }

    fn resolution() -> LedgerEntry {
        // A hashed-looking id and a redaction stamp, as a copied pending line carries.
        let hashed = "0123456789abcdef0123456789abcdef";
        let mut v: serde_json::Value =
            serde_json::from_str(&line(hashed, "feat/x", false, Some("2026-01-01T00:00:00Z")))
                .unwrap();
        v["resolved"] = serde_json::json!({ "from": "main" });
        v["redaction"] = "fp123".into();
        v["session_id"] = hashed.into();
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_prebuilt_entry_is_written_verbatim_and_stamped_and_twice_is_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("l.jsonl");
        let e = resolution();
        let out = append_entries(&p, std::slice::from_ref(&e), &At).unwrap();
        assert_eq!((out.appended, out.skipped), (1, 0));
        let again = append_entries(&p, std::slice::from_ref(&e), &At).unwrap();
        assert_eq!((again.appended, again.skipped), (0, 1));
        let l = read(&p).unwrap();
        assert_eq!(l.entries().len(), 1);
        let got = &l.entries()[0];
        assert_eq!(got.request_id, e.request_id, "ids are not hashed again");
        assert_eq!(got.session_id, e.session_id);
        assert_eq!(got.redaction.as_deref(), Some("fp123"));
        assert_eq!(got.resolved, e.resolved);
        assert_eq!(got.recorded_at.as_deref(), Some("2026-03-01T00:00:00Z"));
    }

    #[test]
    fn the_default_form_needs_tally_and_otherwise_does_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let e = resolution();
        let absent = append_entries_default(dir.path(), std::slice::from_ref(&e), &At).unwrap();
        assert!(absent.tally_absent);
        assert!(!dir.path().join(".tally").exists());
        std::fs::create_dir(dir.path().join(".tally")).unwrap();
        let out = append_entries_default(dir.path(), &[e.clone(), e.clone()], &At).unwrap();
        assert_eq!((out.appended, out.skipped), (1, 1));
        assert_eq!(
            read_default(dir.path()).unwrap().entries()[0].request_id,
            e.request_id
        );
    }
}
