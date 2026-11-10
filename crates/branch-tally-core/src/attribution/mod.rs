//! Assigns each record the branch it belongs to.
//!
//! Ports worktree recovery from `load_records` and the strict filter or session claim from
//! `select` in `reference/token_report.py`, bug-for-bug (including the #6 worktree roll-up
//! behaviour). Consumes [`crate::model::Record`]; hands on [`crate::model::AttributedRecord`]
//! and [`crate::model::SelectedRecord`].

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::model::{
    AgentId, AttributedRecord, BranchBasis, Dispatch, Origin, Record, SelectedRecord, Selection,
    SessionId, SubagentTags, Workspace,
};
use crate::paths;

/// The branch the reference reports for a record with no stamp.
const NONE: &str = "(none)";

/// What the reference's `worktree_branches` builds: worktree identity to the branch it holds.
///
/// Keyed two ways in one map, exactly as the reference does: the lowercased transcript slug of
/// each worktree path, and `worktree-<final path component>` (the harness-generated branch name
/// a subagent dispatch stamps). The main checkout's own entry is in the map too, as git lists it,
/// but attribution never looks it up (#37).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeMap {
    by_key: HashMap<String, String>,
    main_components: Vec<String>,
}

impl WorktreeMap {
    /// Builds the map. `main_checkout` is the main checkout's path and `worktrees` every
    /// `(path, branch)` pair git reports (detached worktrees have no branch and are omitted by
    /// the caller). Paths should already be resolved (absolute, as git reports them).
    pub fn new<'a>(
        main_checkout: &Path,
        worktrees: impl IntoIterator<Item = (&'a Path, &'a str)>,
    ) -> Self {
        let mut by_key = HashMap::new();
        for (path, branch) in worktrees {
            // Every slug form the transcript directory or cwd may use for this path (literal and
            // Git Bash `/d/...` vs `D:` drive), so a lookup hits whichever form arrives.
            for slug in paths::slug_candidates(path) {
                by_key.insert(slug.to_lowercase(), branch.to_owned());
            }
            by_key.insert(
                format!("worktree-{}", final_component(path)),
                branch.to_owned(),
            );
        }
        Self {
            by_key,
            main_components: components(&paths::normalize(main_checkout)),
        }
    }

    /// The branch held under `key`, if any.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.by_key.get(key).map(String::as_str)
    }
}

/// Python's `Path.name`: the last non-empty component, splitting on either separator so a
/// Windows path reads the same on every platform.
fn final_component(path: &Path) -> String {
    paths::normalize(path)
        .rsplit(['/', '\\'])
        .find(|c| !c.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// `FEATURE_RE`: `^(feat|feature|fix|bugfix|hotfix|chore|release)/`.
fn is_feature(branch: &str) -> bool {
    [
        "feat/", "feature/", "fix/", "bugfix/", "hotfix/", "chore/", "release/",
    ]
    .iter()
    .any(|p| branch.starts_with(p))
}

/// Assigns each record a branch (`load_records`' branch step). Order is preserved; no record is
/// dropped. `strict` disables all recovery.
#[must_use]
pub fn attribute(
    records: impl IntoIterator<Item = Record>,
    dispatches: &[Dispatch],
    map: &WorktreeMap,
    strict: bool,
) -> Vec<AttributedRecord> {
    let mut links: HashMap<(&SessionId, &AgentId), &Dispatch> = HashMap::new();
    for d in dispatches {
        links.entry((&d.session_id, &d.agent_id)).or_insert(d);
    }
    records
        .into_iter()
        .map(|record| {
            let stamp = record
                .git_branch
                .as_deref()
                .filter(|b| !b.is_empty())
                .unwrap_or(NONE);
            let (branch, basis, subagent) = if strict {
                (stamp.to_owned(), BranchBasis::Stamped, None)
            } else if let Some(parent) = &record.parent_session {
                roll_up(&record, parent, stamp, &links, map)
            } else {
                let (branch, basis) = recover(&record, stamp, map);
                (branch, basis, None)
            };
            AttributedRecord {
                record,
                branch,
                basis,
                subagent,
            }
        })
        .collect()
}

/// Claim-mode handling of a record from a subagent file. The branch is the dispatcher's raw
/// stamp at dispatch time when the dispatch link is found and that stamp is usable; otherwise
/// the record keeps its raw stamp and is [`BranchBasis::SubagentUnlinked`]. Neither path
/// consults the live worktree list.
fn roll_up(
    record: &Record,
    parent: &SessionId,
    stamp: &str,
    links: &HashMap<(&SessionId, &AgentId), &Dispatch>,
    map: &WorktreeMap,
) -> (String, BranchBasis, Option<SubagentTags>) {
    let tags = subagent_tags(record, map);
    let dispatcher_branch = record
        .agent_id
        .as_ref()
        .and_then(|id| links.get(&(parent, id)))
        .and_then(|d| d.git_branch.as_deref())
        .filter(|b| !b.is_empty() && !b.starts_with("worktree-agent-"));
    match dispatcher_branch {
        Some(branch) => (branch.to_owned(), BranchBasis::DispatcherRolledUp, tags),
        None => (stamp.to_owned(), BranchBasis::SubagentUnlinked, tags),
    }
}

/// Workspace and subagent tags for a record from a subagent file (shared by claim and timeline).
/// The map supplies only the main checkout's path; no branch is recovered from it.
fn subagent_tags(record: &Record, map: &WorktreeMap) -> Option<SubagentTags> {
    record.agent_id.as_ref().map(|id| SubagentTags {
        workspace: record
            .cwd
            .as_deref()
            .and_then(|cwd| workspace_of(cwd, map))
            .unwrap_or(Workspace::Unknown),
        subagent: record
            .attribution
            .agent
            .clone()
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| id.to_string()),
        agent_id: id.clone(),
    })
}

/// Path components for comparison: either separator, a Git-Bash `/d/...` drive form read as
/// `d:`, empty components dropped. Case is kept; callers compare case-insensitively.
fn components(text: &str) -> Vec<String> {
    let text = text.replace('\\', "/");
    let mut parts: Vec<String> = text
        .split('/')
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    if text.starts_with('/')
        && parts.len() > 1
        && parts[0].len() == 1
        && parts[0].chars().all(|c| c.is_ascii_alphabetic())
    {
        parts[0].push(':');
    }
    parts
}

/// Where a subagent ran, from its raw cwd: a worktree named by the directory under
/// `.claude/worktrees/`, or the main checkout. `None` when the cwd is outside both (never
/// guessed as main; the caller tags it [`Workspace::Unknown`]).
fn workspace_of(cwd: &str, map: &WorktreeMap) -> Option<Workspace> {
    let parts = components(&paths::normalize(Path::new(cwd)));
    let worktree = parts.windows(3).find_map(|w| {
        (w[0].eq_ignore_ascii_case(".claude") && w[1].eq_ignore_ascii_case("worktrees"))
            .then(|| w[2].clone())
    });
    if let Some(name) = worktree {
        return Some(Workspace::Worktree(name));
    }
    let main = &map.main_components;
    let inside = !main.is_empty()
        && parts.len() >= main.len()
        && parts
            .iter()
            .zip(main)
            .all(|(a, b)| a.eq_ignore_ascii_case(b));
    inside.then_some(Workspace::Main)
}

/// Non-strict recovery: a known `worktree-agent-` stamp, else `(none)` plus a worktree-origin
/// lookup.
fn recover(record: &Record, stamp: &str, map: &WorktreeMap) -> (String, BranchBasis) {
    if stamp.starts_with("worktree-agent-")
        && let Some(branch) = map.get(stamp)
    {
        return (branch.to_owned(), BranchBasis::WorktreeRecovered);
    }
    // Only a worktree root names a branch for its records. The main checkout's entry in the
    // worktree list is what it has checked out *now*, not when the call was made, so an unstamped
    // main-checkout record stays `(none)` (#37; the reference recovers it).
    if stamp == NONE
        && let Some(origin @ Origin::Worktree(_)) = &record.origin
        && let Some(branch) = map.get(&origin.label())
    {
        return (branch.to_owned(), BranchBasis::OriginRecovered);
    }
    (stamp.to_owned(), BranchBasis::Stamped)
}

/// Picks the records belonging to `branch` (`select`). Order is preserved.
///
/// Strict keeps `assigned == branch`. Otherwise a session is claimed when its only work-shaped
/// branch is `branch`, and all its records are kept ([`Selection::SessionClaim`] unless the
/// record itself is on `branch`).
#[must_use]
pub fn select(records: Vec<AttributedRecord>, branch: &str, strict: bool) -> Vec<SelectedRecord> {
    let claimed: HashSet<SessionId> = if strict {
        HashSet::new()
    } else {
        let mut features: HashMap<&SessionId, HashSet<&str>> = HashMap::new();
        for r in &records {
            if is_feature(&r.branch) {
                features
                    .entry(&r.record.session_id)
                    .or_default()
                    .insert(&r.branch);
            }
        }
        features
            .into_iter()
            .filter(|(_, brs)| brs.len() == 1 && brs.contains(branch))
            .map(|(s, _)| s.clone())
            .collect()
    };
    records
        .into_iter()
        .filter_map(|attributed| {
            let kept_by = if attributed.branch == branch {
                Selection::Exact
            } else if claimed.contains(&attributed.record.session_id) {
                Selection::SessionClaim
            } else {
                return None;
            };
            Some(SelectedRecord {
                attributed,
                kept_by,
            })
        })
        .collect()
}

/// The default work-branch predicate for timeline mode (the CLI's `--work-branch` overrides it).
///
/// A branch is a work branch unless it is the repository's default branch (`default_branch`,
/// from `Git::default_branch`), a harness-generated `worktree-agent-<id>` branch, the literal
/// `HEAD` (a detached checkout) or empty. Everything else counts, so `feat/`, `fix/`,
/// `refactor/`, `docs/` and any other naming scheme are work.
pub fn default_work_branch(default_branch: &str) -> impl Fn(&str) -> bool + use<> {
    let default_branch = default_branch.to_owned();
    move |branch: &str| {
        !branch.is_empty()
            && branch != "HEAD"
            && branch != default_branch
            && !branch.starts_with(WORKTREE_AGENT)
    }
}

/// The branch name the harness generates for a subagent worktree.
const WORKTREE_AGENT: &str = "worktree-agent-";

/// Nanoseconds since the Unix epoch for an RFC 3339 timestamp with an explicit offset
/// (`2026-11-01T10:00:00.000Z`, `...+01:00`). `None` for anything else: a timestamp whose
/// instant cannot be known is never guessed.
fn parse_ts(text: &str) -> Option<i128> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let digits = b.get(from..from + len)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| digits.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    };
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    if !matches!(b.get(10), Some(b'T' | b't' | b' ')) || b.get(13) != Some(&b':') {
        return None;
    }
    if b.get(16) != Some(&b':') {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut at = 19;
    let mut nanos: i64 = 0;
    if b.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while b.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return None;
        }
        let mut scale = 100_000_000;
        for d in &b[start..at.min(start + 9)] {
            nanos += i64::from(d - b'0') * scale;
            scale /= 10;
        }
    }
    let offset_secs = match b.get(at)? {
        b'Z' | b'z' if at + 1 == b.len() => 0,
        sign @ (b'+' | b'-') => {
            let (oh, om) = match b.len() - at - 1 {
                2 => (num(at + 1, 2)?, 0),
                4 => (num(at + 1, 2)?, num(at + 3, 2)?),
                5 if b.get(at + 3) == Some(&b':') => (num(at + 1, 2)?, num(at + 4, 2)?),
                _ => return None,
            };
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = oh * 3600 + om * 60;
            if *sign == b'-' { -secs } else { secs }
        }
        _ => return None,
    };
    // Days from civil (proleptic Gregorian), Howard Hinnant's algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    Some(i128::from(secs) * 1_000_000_000 + i128::from(nanos))
}

/// Where a non-work moment falls on a session's timeline.
enum Placement {
    /// On this work branch; `forward` when no work branch was active yet.
    Branch { branch: String, forward: bool },
    /// The moment coincides with records on conflicting work branches.
    Ambiguous,
    /// The timeline has no usable work branch to place it on.
    NoWork,
}

/// All records of one session sharing a timestamp and a work branch (or in conflict).
struct Cut {
    ts: i128,
    branch: String,
    conflict: bool,
}

/// One session's work-branch cuts in time order, built from its own `(ts, gitBranch)` pairs.
#[derive(Default)]
struct Timeline {
    cuts: Vec<Cut>,
    /// Any top-level record in the session stamped a work branch (even one with no usable ts).
    has_work_stamp: bool,
}

impl Timeline {
    fn add(&mut self, ts: Option<i128>, work: Option<&str>) {
        let Some(branch) = work else { return };
        self.has_work_stamp = true;
        let Some(ts) = ts else { return };
        match self.cuts.iter_mut().find(|c| c.ts == ts) {
            Some(cut) => cut.conflict |= cut.branch != branch,
            None => self.cuts.push(Cut {
                ts,
                branch: branch.to_owned(),
                conflict: false,
            }),
        }
    }

    fn finish(&mut self) {
        self.cuts.sort_by_key(|c| c.ts);
    }

    fn is_conflict(&self, ts: i128) -> bool {
        self.cuts.iter().any(|c| c.ts == ts && c.conflict)
    }

    /// The branch of the last undisputed cut at or before `ts`, else the first one after it.
    fn place(&self, ts: i128) -> Placement {
        if self.is_conflict(ts) {
            return Placement::Ambiguous;
        }
        let mut usable = self.cuts.iter().filter(|c| !c.conflict);
        let before = usable.clone().take_while(|c| c.ts <= ts).last();
        match (before, usable.next()) {
            (Some(cut), first) => Placement::Branch {
                branch: cut.branch.clone(),
                forward: first.is_some_and(|f| f.ts == cut.ts) && cut.ts == ts,
            },
            (None, Some(first)) => Placement::Branch {
                branch: first.branch.clone(),
                forward: true,
            },
            (None, None) => Placement::NoWork,
        }
    }
}

/// Timeline attribution: each session's records are placed on the work branch that was active at
/// their own timestamp, from the session's own `(ts, gitBranch)` pairs. Order is preserved; no
/// record is dropped.
///
/// `is_work` decides which stamps are work branches; it sees only non-empty stamps. A
/// `worktree-agent-<id>` stamp is never a work branch whatever `is_work` says. A non-work record
/// goes back to the last active work branch ([`BranchBasis::TimelineCarried`]), or forward to the
/// session's first one ([`BranchBasis::TimelineForward`]). A session with no work branch keeps
/// every stamp ([`BranchBasis::Stamped`]). A record whose place cannot be decided (no parseable
/// timestamp; the same instant on different work branches) is [`BranchBasis::TimelineAmbiguous`]
/// and keeps its raw stamp.
///
/// A subagent record (`parent_session` set) takes the dispatcher session's timeline branch at the
/// dispatch timestamp ([`BranchBasis::DispatcherRolledUp`]); with no usable link it keeps its raw
/// stamp as [`BranchBasis::SubagentUnlinked`]. No worktree branch is looked up: `map` only supplies
/// the main checkout path for the subagent's workspace tag.
#[must_use]
pub fn attribute_timeline(
    records: impl IntoIterator<Item = Record>,
    dispatches: &[Dispatch],
    map: &WorktreeMap,
    is_work: &dyn Fn(&str) -> bool,
) -> Vec<AttributedRecord> {
    let records: Vec<Record> = records.into_iter().collect();
    let work_of = |record: &Record| -> Option<String> {
        record
            .git_branch
            .as_deref()
            .filter(|b| !b.is_empty() && !b.starts_with(WORKTREE_AGENT) && is_work(b))
            .map(str::to_owned)
    };
    let ts_of = |record: &Record| record.ts.as_deref().and_then(parse_ts);

    let mut timelines: HashMap<&SessionId, Timeline> = HashMap::new();
    for record in records.iter().filter(|r| r.parent_session.is_none()) {
        timelines
            .entry(&record.session_id)
            .or_default()
            .add(ts_of(record), work_of(record).as_deref());
    }
    timelines.values_mut().for_each(Timeline::finish);

    let mut links: HashMap<(&SessionId, &AgentId), &Dispatch> = HashMap::new();
    for d in dispatches {
        links.entry((&d.session_id, &d.agent_id)).or_insert(d);
    }

    records
        .iter()
        .map(|record| {
            let stamp = record
                .git_branch
                .as_deref()
                .filter(|b| !b.is_empty())
                .unwrap_or(NONE);
            let (branch, basis, subagent) = if let Some(parent) = &record.parent_session {
                let placed = record
                    .agent_id
                    .as_ref()
                    .and_then(|id| links.get(&(parent, id)))
                    .and_then(|d| d.ts.as_deref().and_then(parse_ts))
                    .zip(timelines.get(parent))
                    .map(|(ts, timeline)| timeline.place(ts));
                let tags = subagent_tags(record, map);
                match placed {
                    Some(Placement::Branch { branch, .. }) => {
                        (branch, BranchBasis::DispatcherRolledUp, tags)
                    }
                    _ => (stamp.to_owned(), BranchBasis::SubagentUnlinked, tags),
                }
            } else {
                let (branch, basis) =
                    place_record(record, stamp, &timelines, ts_of(record), &work_of);
                (branch, basis, None)
            };
            AttributedRecord {
                record: record.clone(),
                branch,
                basis,
                subagent,
            }
        })
        .collect()
}

/// Places one top-level record on its session's timeline.
fn place_record(
    record: &Record,
    stamp: &str,
    timelines: &HashMap<&SessionId, Timeline>,
    ts: Option<i128>,
    work_of: &dyn Fn(&Record) -> Option<String>,
) -> (String, BranchBasis) {
    let ambiguous = || (stamp.to_owned(), BranchBasis::TimelineAmbiguous);
    let Some(timeline) = timelines.get(&record.session_id) else {
        return (stamp.to_owned(), BranchBasis::Stamped);
    };
    if !timeline.has_work_stamp {
        return (stamp.to_owned(), BranchBasis::Stamped);
    }
    let Some(ts) = ts else { return ambiguous() };
    if let Some(own) = work_of(record) {
        return if timeline.is_conflict(ts) {
            ambiguous()
        } else {
            (own, BranchBasis::Stamped)
        };
    }
    match timeline.place(ts) {
        Placement::Branch { branch, forward } => {
            let basis = if forward {
                BranchBasis::TimelineForward
            } else {
                BranchBasis::TimelineCarried
            };
            (branch, basis)
        }
        Placement::Ambiguous | Placement::NoWork => ambiguous(),
    }
}

/// Picks the records whose timeline-assigned branch is `branch`, all [`Selection::Exact`]. There
/// is no session claim in timeline mode. Order is preserved.
#[must_use]
pub fn select_timeline(records: Vec<AttributedRecord>, branch: &str) -> Vec<SelectedRecord> {
    records
        .into_iter()
        .filter(|a| a.branch == branch)
        .map(|attributed| SelectedRecord {
            attributed,
            kept_by: Selection::Exact,
        })
        .collect()
}
