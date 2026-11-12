use super::*;
use crate::model::{Mode, Origin, Workspace};

const CLOCK: Fixed = Fixed("2026-01-01T00:00:00Z");

fn selected(id: &str, branch: &str, output: u64) -> SelectedRecord {
    let mut r = Record::new(
        Harness::ClaudeCode,
        SessionId::new("sess-1"),
        RequestId::new(id),
        Usage {
            input: 1,
            cache_write_5m: 2,
            cache_write_1h: 3,
            cache_read: 4,
            output,
            thinking: Some(1),
        },
    );
    r.agent_id = Some(AgentId::new("agent-1"));
    r.ts = Some("2026-01-01T00:00:00Z".to_owned());
    r.model = Some("claude-x".to_owned());
    r.cwd = Some("/secret/cwd".to_owned());
    r.git_branch = Some("raw-stamp".to_owned());
    r.origin = Some(Origin::Main);
    r.attribution = Attribution {
        skill: Some("sk".to_owned()),
        agent: Some("ag".to_owned()),
    };
    SelectedRecord {
        attributed: AttributedRecord {
            record: r,
            branch: branch.to_owned(),
            basis: BranchBasis::DispatcherRolledUp,
            subagent: Some(SubagentTags {
                workspace: Workspace::worktree("wt"),
                subagent: "ag".to_owned(),
                agent_id: AgentId::new("agent-1"),
            }),
        },
        kept_by: Selection::SessionClaim,
    }
}

fn path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(".tally").join("ledger.jsonl")
}

#[test]
fn entry_round_trips_stored_fields_and_drops_the_rest() {
    let s = selected("r1", "feat/a", 5);
    let back = LedgerEntry::from_selected(&s, "b", Mode::Claim, None).to_selected(None);
    let (a, b) = (&s.attributed, &back.attributed);
    assert_eq!(b.branch, "feat/a");
    assert_eq!(b.basis, a.basis);
    assert_eq!(b.subagent, a.subagent);
    assert_eq!(back.kept_by, s.kept_by);
    assert_eq!(b.record.usage, a.record.usage);
    assert_eq!(b.record.request_id, a.record.request_id);
    assert_eq!(b.record.session_id, a.record.session_id);
    assert_eq!(b.record.agent_id, a.record.agent_id);
    assert_eq!(b.record.ts, a.record.ts);
    assert_eq!(b.record.model, a.record.model);
    assert_eq!(b.record.attribution, a.record.attribution);
    assert_eq!(b.record.cwd, None);
    assert_eq!(b.record.git_branch, None);
    assert_eq!(b.record.origin, a.record.origin);
}

#[test]
fn missing_file_is_an_empty_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let l = read(&path(&dir)).unwrap();
    assert!(l.is_empty());
    assert_eq!(*l.stats(), ReadStats::default());
}

#[test]
fn append_creates_dir_writes_lf_lines_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let recs = [selected("r1", "b", 1), selected("r2", "b", 2)];
    let out = append(&p, &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!(
        out,
        AppendOutcome {
            appended: 2,
            skipped: 0,
            unnamed_origins: 0,
            tally_absent: false,
        }
    );
    let text = std::fs::read_to_string(&p).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(text.ends_with('\n') && !text.contains('\r'));
    let again = append(&p, &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!(
        again,
        AppendOutcome {
            appended: 0,
            skipped: 2,
            unnamed_origins: 0,
            tally_absent: false,
        }
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
}

#[test]
fn append_skips_duplicate_ids_within_one_call() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let recs = [selected("r1", "b", 1), selected("r1", "b", 1)];
    let out = append(&p, &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!(
        out,
        AppendOutcome {
            appended: 1,
            skipped: 1,
            unnamed_origins: 0,
            tally_absent: false,
        }
    );
}

#[test]
fn lines_have_stable_key_order() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    append(
        &p,
        &[selected("r1", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(
        text.starts_with(
            r#"{"v":1,"request_id":"r1","harness":"claude-code","session_id":"sess-1""#
        )
    );
}

#[test]
fn append_after_a_file_without_trailing_newline_starts_a_new_line() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    append(
        &p,
        &[selected("r1", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, text.trim_end()).unwrap();
    append(
        &p,
        &[selected("r2", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let l = read(&p).unwrap();
    assert_eq!(l.entries().len(), 2);
    assert_eq!(l.stats().malformed, 0);
}

#[test]
fn malformed_and_unsupported_lines_are_counted_with_line_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    append(
        &p,
        &[selected("r1", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let mut text = std::fs::read(&p).unwrap();
    text.extend_from_slice(b"not json at all\n");
    text.extend_from_slice(b"{\"v\":1}\n");
    text.extend_from_slice(b"{\"v\":99,\"request_id\":\"x\"}\n");
    text.extend_from_slice(&[0xff, 0xfe, b'\n']);
    text.extend_from_slice(b"\n");
    std::fs::write(&p, text).unwrap();
    let l = read(&p).unwrap();
    let s = l.stats();
    assert_eq!(l.entries().len(), 1);
    assert_eq!((s.lines, s.malformed, s.unsupported_version), (5, 3, 1));
    assert_eq!(s.bad_line_numbers, vec![2, 3, 4, 5]);
}

#[test]
fn identical_duplicates_collapse_and_are_counted_as_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    append(
        &p,
        &[selected("r1", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, format!("{text}{text}")).unwrap();
    let l = read(&p).unwrap();
    let s = l.stats();
    assert_eq!((l.entries().len(), s.duplicates, s.conflicts), (1, 1, 0));
}

#[test]
fn conflicting_duplicates_resolve_deterministically_and_are_counted() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let a = serde_json::to_string(&LedgerEntry::from_selected(
        &selected("r1", "b", 1),
        "b",
        Mode::Claim,
        None,
    ))
    .unwrap();
    let b = serde_json::to_string(&LedgerEntry::from_selected(
        &selected("r1", "b", 9),
        "b",
        Mode::Claim,
        None,
    ))
    .unwrap();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, format!("{a}\n{b}\n")).unwrap();
    let one = read(&p).unwrap();
    std::fs::write(&p, format!("{b}\n{a}\n")).unwrap();
    let two = read(&p).unwrap();
    assert_eq!(one.stats().conflicts, 1);
    assert_eq!(one.entries().len(), 1);
    assert_eq!(one.entries(), two.entries());
}

struct Mask;
impl Redact for Mask {
    fn redact(&self, mut e: LedgerEntry) -> LedgerEntry {
        e.session_id = SessionId::new("masked");
        e
    }
}

#[test]
fn redaction_applies_to_every_entry_before_write() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    append(
        &p,
        &[selected("r1", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Mask,
        &CLOCK,
    )
    .unwrap();
    let l = read(&p).unwrap();
    assert_eq!(l.entries()[0].session_id.as_str(), "masked");
}

#[test]
fn union_counts_each_side_and_prefers_the_ledger_copy() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    append(
        &p,
        &[selected("r1", "b", 1), selected("r2", "b", 2)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &CLOCK,
    )
    .unwrap();
    let l = read(&p).unwrap();
    let live = [
        selected("r2", "b", 2),
        selected("r3", "b", 3),
        selected("r3", "b", 3),
    ];
    let u = union(&l, &live, None);
    assert_eq!((u.ledger_only, u.live_only, u.in_both), (1, 1, 1));
    let ids: Vec<&str> = u
        .records
        .iter()
        .map(|s| s.attributed.record.request_id.as_str())
        .collect();
    assert_eq!(ids, ["r1", "r2", "r3"]);
    assert_eq!(u.records[1].attributed.record.cwd, None);
}

struct Fixed(&'static str);
impl Clock for Fixed {
    fn now(&self) -> String {
        self.0.to_owned()
    }
}

fn line(id: &str, output: u64, recorded_at: Option<&str>) -> String {
    let mut e = LedgerEntry::from_selected(&selected(id, "b", output), "b", Mode::Claim, None);
    e.recorded_at = recorded_at.map(str::to_owned);
    serde_json::to_string(&e).unwrap()
}

fn write_lines(p: &Path, lines: &[String]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, format!("{}\n", lines.join("\n"))).unwrap();
}

#[test]
fn append_stamps_recorded_at_from_the_injected_clock() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let clock = Fixed("2026-02-03T04:05:06Z");
    append(
        &p,
        &[selected("r1", "b", 1)],
        "b",
        Mode::Claim,
        None,
        &Identity,
        &clock,
    )
    .unwrap();
    let l = read(&p).unwrap();
    assert_eq!(
        l.entries()[0].recorded_at.as_deref(),
        Some("2026-02-03T04:05:06Z")
    );
}

#[test]
fn system_clock_formats_rfc3339_utc() {
    assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(format_rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
    let now = SystemClock.now();
    assert_eq!((now.len(), now.ends_with('Z')), (20, true));
}

#[test]
fn lines_differing_only_in_recorded_at_are_duplicates_not_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    write_lines(
        &p,
        &[
            line("r1", 1, Some("2026-01-01T00:00:00Z")),
            line("r1", 1, Some("2026-01-02T00:00:00Z")),
        ],
    );
    let l = read(&p).unwrap();
    let s = l.stats();
    assert_eq!((l.entries().len(), s.duplicates, s.conflicts), (1, 1, 0));
}

#[test]
fn a_conflict_is_won_by_the_latest_recorded_at_in_either_order() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    // The newer line has the larger serialized form, so the old smallest-line rule would lose.
    let old = line("r1", 1, Some("2026-01-01T00:00:00Z"));
    let new = line("r1", 9, Some("2026-01-02T00:00:00Z"));
    for order in [[&old, &new], [&new, &old]] {
        write_lines(&p, &[order[0].clone(), order[1].clone()]);
        let l = read(&p).unwrap();
        assert_eq!(l.stats().conflicts, 1);
        assert_eq!(l.entries()[0].usage.output, 9);
    }
}

#[test]
fn a_tie_or_a_missing_recorded_at_falls_back_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let t = Some("2026-01-01T00:00:00Z");
    // Tie: the smallest serialized line wins.
    write_lines(&p, &[line("r1", 9, t), line("r1", 1, t)]);
    let l = read(&p).unwrap();
    assert_eq!((l.stats().conflicts, l.entries()[0].usage.output), (1, 1));
    // Missing reads as oldest: the stamped line wins even though it sorts later.
    write_lines(&p, &[line("r2", 1, None), line("r2", 9, t)]);
    let l = read(&p).unwrap();
    assert_eq!((l.stats().conflicts, l.entries()[0].usage.output), (1, 9));
    // Both missing: smallest line.
    write_lines(&p, &[line("r3", 9, None), line("r3", 1, None)]);
    assert_eq!(read(&p).unwrap().entries()[0].usage.output, 1);
}

#[test]
fn a_line_without_recorded_at_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    write_lines(&p, &[line("r1", 1, None)]);
    assert!(!std::fs::read_to_string(&p).unwrap().contains("recorded_at"));
    let l = read(&p).unwrap();
    assert_eq!((l.entries().len(), l.stats().malformed), (1, 0));
    assert_eq!(l.entries()[0].recorded_at, None);
}

#[test]
fn the_same_request_recorded_for_two_branches_keeps_one_line_each() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let recs = [selected("r1", "other", 1)];
    let a = append(&p, &recs, "feat/a", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    let b = append(&p, &recs, "feat/b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!((a.appended, b.appended), (1, 1));
    // Re-recording either one appends nothing.
    let a2 = append(&p, &recs, "feat/a", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    let b2 = append(&p, &recs, "feat/b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!(
        (a2.appended, a2.skipped, b2.appended, b2.skipped),
        (0, 1, 0, 1)
    );
    let l = read(&p).unwrap();
    assert_eq!(l.entries().len(), 2);
    assert_eq!((l.stats().duplicates, l.stats().conflicts), (0, 0));
}

#[test]
fn for_branch_keeps_only_entries_recorded_for_that_branch() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let one = [selected("r1", "x", 1)];
    let two = [selected("r1", "x", 1), selected("r2", "x", 2)];
    append(&p, &one, "feat/a", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    append(&p, &two, "feat/b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    let l = read(&p).unwrap();
    assert_eq!(l.for_branch("feat/a").entries().len(), 1);
    assert_eq!(l.for_branch("feat/b").entries().len(), 2);
    assert_eq!(l.for_branch("nope").entries().len(), 0);
}

#[test]
fn the_same_key_with_a_different_mode_is_a_conflict_won_by_the_later_write() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let recs = [selected("r1", "x", 1)];
    let early = Fixed("2026-01-01T00:00:00Z");
    append(&p, &recs, "feat/a", Mode::Claim, None, &Identity, &early).unwrap();
    // Appending skips the held key, so write the second line directly.
    let mut later = LedgerEntry::from_selected(&recs[0], "feat/a", Mode::Timeline, None);
    later.recorded_at = Some("2026-02-01T00:00:00Z".to_owned());
    let mut text = std::fs::read_to_string(&p).unwrap();
    text.push_str(&serde_json::to_string(&later).unwrap());
    text.push('\n');
    std::fs::write(&p, text).unwrap();
    let l = read(&p).unwrap();
    assert_eq!((l.entries().len(), l.stats().conflicts), (1, 1));
    assert_eq!(l.entries()[0].mode, Mode::Timeline);
}

#[test]
fn modes_lists_each_distinct_mode_once_in_stable_order() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    let at = |id: &str, mode| {
        append(
            &p,
            &[selected(id, "x", 1)],
            "feat/a",
            mode,
            None,
            &Identity,
            &CLOCK,
        )
    };
    at("r1", Mode::Timeline).unwrap();
    at("r2", Mode::Claim).unwrap();
    at("r3", Mode::Claim).unwrap();
    let l = read(&p).unwrap();
    assert_eq!(l.modes(), vec![Mode::Claim, Mode::Timeline]);
}

/// A checkout that has opted in: `.tally/` exists.
fn opted_in() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(TALLY_DIR)).unwrap();
    dir
}

#[test]
fn default_append_and_read_round_trip_under_an_existing_tally_dir() {
    let dir = opted_in();
    let recs = [selected("r1", "b", 1), selected("r2", "b", 2)];
    let out = append_default(dir.path(), &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!((out.appended, out.skipped, out.tally_absent), (2, 0, false));
    let again =
        append_default(dir.path(), &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!((again.appended, again.skipped), (0, 2));
    assert_eq!(read_default(dir.path()).unwrap().entries().len(), 2);
}

#[test]
fn default_append_with_no_tally_dir_writes_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let recs = [selected("r1", "b", 1)];
    let out = append_default(dir.path(), &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!((out.appended, out.skipped, out.tally_absent), (0, 0, true));
    assert!(!dir.path().join(TALLY_DIR).exists());
    assert!(read_default(dir.path()).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn a_planted_symlink_at_the_ledger_is_not_followed() {
    let dir = opted_in();
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"precious").unwrap();
    std::os::unix::fs::symlink(&victim, path(&dir)).unwrap();
    let recs = [selected("r1", "b", 1)];
    let err =
        append_default(dir.path(), &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap_err();
    assert!(matches!(err, LedgerError::Refused { .. }), "{err}");
    assert!(matches!(
        read_default(dir.path()).unwrap_err(),
        LedgerError::Refused { .. }
    ));
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_at_the_ledger_is_not_created() {
    let dir = opted_in();
    let target = dir.path().join("created-by-attacker");
    std::os::unix::fs::symlink(&target, path(&dir)).unwrap();
    let recs = [selected("r1", "b", 1)];
    assert!(append_default(dir.path(), &recs, "b", Mode::Claim, None, &Identity, &CLOCK).is_err());
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_tally_dir_is_refused_for_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    std::os::unix::fs::symlink(&elsewhere, repo.join(TALLY_DIR)).unwrap();
    let recs = [selected("r1", "b", 1)];
    assert!(append_default(&repo, &recs, "b", Mode::Claim, None, &Identity, &CLOCK).is_err());
    assert!(read_default(&repo).is_err());
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn a_directory_or_file_kind_mismatch_at_the_ledger_is_refused() {
    let dir = opted_in();
    std::fs::create_dir(path(&dir)).unwrap();
    assert!(matches!(
        read_default(dir.path()).unwrap_err(),
        LedgerError::Refused { .. }
    ));
}

#[cfg(unix)]
#[test]
fn the_explicit_path_keeps_following_a_symlink() {
    // `--ledger <path>` is the user's own choice and keeps today's behaviour.
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.jsonl");
    let link = dir.path().join("link.jsonl");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let recs = [selected("r1", "b", 1)];
    append(&link, &recs, "b", Mode::Claim, None, &Identity, &CLOCK).unwrap();
    assert_eq!(read(&real).unwrap().entries().len(), 1);
}
