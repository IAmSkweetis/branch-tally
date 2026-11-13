//! Aggregates selected records into the by-model, by-attribution and by-session tables.
//!
//! Ports `summarize` from `reference/token_report.py`, plus the numbers its `render` derives
//! (the Total row and the footnote counts). Sits between attribution and pricing and produces
//! [`crate::model::Summary`].
//!
//! All three tables are in first-seen order; sorting is `render`'s job.
//!
//! Python truthiness is mirrored on purpose: an empty skill, agent or model string is treated
//! like an absent one, as the reference's `or` chains do.

use std::collections::{BTreeSet, HashMap};

use crate::model::{
    AttributedRecord, BranchBasis, Bucket, HeuristicCount, Mode, Origin, SelectedRecord, Selection,
    SessionId, SessionRow, SubagentRow, SubagentSummary, SubagentTags, Summary, Unstamped,
};

/// Calls and tokens of every [`BranchBasis::TimelineAmbiguous`] or
/// [`BranchBasis::SubagentUnlinked`] record in `attributed` (selected or not) whose session has
/// at least one record in `selected`. Pass the result to [`summarize`].
#[must_use]
pub fn count_ambiguous(
    attributed: &[AttributedRecord],
    selected: &[SelectedRecord],
) -> HeuristicCount {
    let sessions: std::collections::HashSet<&SessionId> = selected
        .iter()
        .map(|s| &s.attributed.record.session_id)
        .collect();
    let mut count = HeuristicCount::default();
    for a in attributed {
        if matches!(
            a.basis,
            BranchBasis::TimelineAmbiguous | BranchBasis::SubagentUnlinked
        ) && sessions.contains(&a.record.session_id)
        {
            count.add(a.record.usage.tokens());
        }
    }
    count
}

/// The branch attribution gives a record whose transcript carried no `gitBranch` stamp.
const NO_BRANCH: &str = "(none)";

/// Counts the calls no branch's report places (#60): see [`Unstamped`]. Pass the **full
/// attributed set before selection** and the report's `mode`.
#[must_use]
pub fn count_unstamped(attributed: &[AttributedRecord], mode: Mode) -> Unstamped {
    // Only claim mode places a record on a branch it was not assigned (the session claim);
    // strict and timeline selection keep `assigned == branch` only.
    let claims = if mode == Mode::Claim {
        crate::attribution::session_claims(attributed)
    } else {
        HashMap::new()
    };
    attributed
        .iter()
        .filter(|a| {
            a.basis != BranchBasis::SubagentUnlinked
                && a.branch == NO_BRANCH
                && !claims.contains_key(&a.record.session_id)
        })
        .fold(Unstamped::default(), |acc, a| Unstamped {
            calls: acc.calls + 1,
            tokens: acc.tokens + a.record.usage.tokens(),
        })
}

/// The `origin` text used when a record has no origin. The reference always has one, so this can
/// only come from a source that could not tell; it is never counted as a worktree session.
const UNKNOWN_ORIGIN: &str = "unknown";

/// Turns the records selected for `branch` into a [`Summary`].
///
/// `mode` is carried through unchanged. Each record's assigned branch is compared with `branch`
/// to count off-branch tokens.
///
/// `unattributed` is `(calls, tokens)` from [`SubagentSummary::count_unattributed`] over the
/// **full attributed set before selection**, so it is repository-wide: every subagent record the
/// read saw that had no usable dispatch link, not only those in the selected sessions. Render
/// must label it that way. Subagent rows cover only the selected records whose basis is
/// [`BranchBasis::DispatcherRolledUp`]; selected unlinked records are counted into
/// `unlinked_in_total_*` instead (they are in the total via session claim). In
/// [`Mode::Strict`] the subagent breakdown (rows and unattributed) is left empty.
///
/// `footnote.heuristics` counts each selected record under one field, by its basis (a `Stamped`
/// record kept by [`Selection::SessionClaim`] counts as session claim). `ambiguous` comes from
/// [`count_ambiguous`] over the full attributed set and fills `heuristics.ambiguous`. In
/// [`Mode::Strict`], the raw view, the whole breakdown stays at its default.
#[must_use]
pub fn summarize(
    branch: &str,
    mode: Mode,
    selected: &[SelectedRecord],
    unattributed: (u64, u64),
    ambiguous: HeuristicCount,
) -> Summary {
    let mut sub_rows: Vec<SubagentRow> = Vec::new();
    let mut by_model: Vec<(String, Bucket)> = Vec::new();
    let mut model_ix: HashMap<String, usize> = HashMap::new();
    let mut by_attribution: Vec<(String, Bucket)> = Vec::new();
    let mut attr_ix: HashMap<String, usize> = HashMap::new();
    let mut sessions: Vec<(SessionId, SessionRow)> = Vec::new();
    let mut session_ix: HashMap<SessionId, usize> = HashMap::new();
    // Parallel to `sessions`: whether the session's origin is known and not the main checkout.
    let mut is_worktree: Vec<bool> = Vec::new();
    let mut footnote = crate::model::Footnote::default();
    let (mut unlinked_calls, mut unlinked_tokens) = (0_u64, 0_u64);

    for sel in selected {
        let attributed = &sel.attributed;
        let r = &attributed.record;
        let u = &r.usage;

        let model = r
            .model
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or("unknown");
        let attr = attribution_key(r);

        add(slot(&mut by_model, &mut model_ix, model), r);
        add(slot(&mut by_attribution, &mut attr_ix, &attr), r);

        let ts = r.ts.as_deref().unwrap_or("");
        let ix = if let Some(&i) = session_ix.get(&r.session_id) {
            i
        } else {
            let origin = r
                .origin
                .as_ref()
                .map_or_else(|| UNKNOWN_ORIGIN.to_owned(), Origin::label);
            is_worktree.push(r.origin.as_ref().is_some_and(|o| o.label() != "main"));
            sessions.push((r.session_id.clone(), SessionRow::new(ts, origin)));
            session_ix.insert(r.session_id.clone(), sessions.len() - 1);
            sessions.len() - 1
        };
        let row = &mut sessions[ix].1;
        if ts < row.first.as_str() {
            ts.clone_into(&mut row.first);
        }
        if ts > row.last.as_str() {
            ts.clone_into(&mut row.last);
        }
        row.tokens += u.tokens();
        row.calls += 1;
        if let Some(agent) = attr.strip_prefix("agent:") {
            row.agents.insert(agent.to_owned());
        }

        // Only verified roll-ups make breakdown rows; an unlinked record stays in the total via
        // the session claim but is reported separately as unverified.
        match (&attributed.subagent, attributed.basis) {
            (Some(tags), BranchBasis::DispatcherRolledUp) => {
                add_subagent(&mut sub_rows, tags, r, model);
            }
            (_, BranchBasis::SubagentUnlinked) => {
                unlinked_calls += 1;
                unlinked_tokens += u.tokens();
            }
            _ => {}
        }

        if r.is_sidechain {
            footnote.subagent_calls += 1;
        }
        // One field per record. The basis wins over the selection reason; a selected ambiguous or
        // unlinked record is counted by the `ambiguous` argument instead.
        count_heuristic(&mut footnote.heuristics, sel);
        if attributed.branch != branch {
            footnote.off_branch_tokens += u.tokens();
        }
    }
    // Strict is the raw view: no heuristic ran, so there is nothing to break down.
    if mode == Mode::Strict {
        footnote.heuristics = crate::model::HeuristicBreakdown::default();
    } else {
        footnote.heuristics.ambiguous = ambiguous;
    }
    footnote.worktree_sessions = is_worktree.iter().filter(|w| **w).count() as u64;

    let total = sum_buckets(&by_model);

    // Strict mode is the raw view: no roll-up, so no breakdown and nothing to call unattributed.
    let subagents = if mode == Mode::Strict {
        SubagentSummary::default()
    } else {
        SubagentSummary {
            rows: sub_rows,
            unattributed_calls: unattributed.0,
            unattributed_tokens: unattributed.1,
            unlinked_in_total_calls: unlinked_calls,
            unlinked_in_total_tokens: unlinked_tokens,
        }
    };

    Summary {
        branch: branch.to_owned(),
        mode,
        total,
        footnote,
        by_model,
        by_attribution,
        sessions,
        subagents,
        work_branch: None,
        // Repository-wide, so the caller sets it from the full attributed set.
        unstamped: crate::model::Unstamped::default(),
        redacted: false,
    }
}

/// The Total row: every bucket of the by-model table added together.
fn sum_buckets(by_model: &[(String, Bucket)]) -> Bucket {
    let mut total = Bucket::default();
    for (_, b) in by_model {
        total.input += b.input;
        total.cache_write_5m += b.cache_write_5m;
        total.cache_write_1h += b.cache_write_1h;
        total.cache_read += b.cache_read;
        total.output += b.output;
        total.thinking += b.thinking;
        total.calls += b.calls;
    }
    total
}

/// Counts one selected record under the one heuristic that placed it, if any.
fn count_heuristic(h: &mut crate::model::HeuristicBreakdown, sel: &SelectedRecord) {
    let tokens = sel.attributed.record.usage.tokens();
    match sel.attributed.basis {
        BranchBasis::WorktreeRecovered => h.worktree_recovery.add(tokens),
        BranchBasis::OriginRecovered => h.origin_recovery.add(tokens),
        BranchBasis::DispatcherRolledUp => h.dispatcher_roll_up.add(tokens),
        BranchBasis::TimelineForward => h.timeline_forward.add(tokens),
        BranchBasis::TimelineCarried => h.timeline_carried.add(tokens),
        BranchBasis::TimelineAmbiguous | BranchBasis::SubagentUnlinked => {}
        BranchBasis::Captured => h.captured.add(tokens),
        BranchBasis::Stamped => {
            if sel.kept_by == Selection::SessionClaim {
                h.session_claim.add(tokens);
            }
        }
    }
}

/// Adds one call to the (workspace, subagent, model) row, appending it in first-seen order when
/// new.
fn add_subagent(
    rows: &mut Vec<SubagentRow>,
    tags: &SubagentTags,
    r: &crate::model::Record,
    model: &str,
) {
    // Linear scan: distinct (workspace, subagent, model) keys are few.
    let i = rows
        .iter()
        .position(|s| {
            s.workspace == tags.workspace && s.subagent == tags.subagent && s.model == model
        })
        .unwrap_or_else(|| {
            rows.push(SubagentRow {
                workspace: tags.workspace.clone(),
                subagent: tags.subagent.clone(),
                model: model.to_owned(),
                calls: 0,
                tokens: 0,
                agent_ids: BTreeSet::new(),
                by_model: Vec::new(),
            });
            rows.len() - 1
        });
    rows[i].calls += 1;
    rows[i].tokens += r.usage.tokens();
    rows[i].agent_ids.insert(tags.agent_id.clone());
    // Same accumulation as the by-model table, per row, so the two cannot drift apart.
    let by_model = &mut rows[i].by_model;
    let m = by_model
        .iter()
        .position(|(n, _)| n == model)
        .unwrap_or_else(|| {
            by_model.push((model.to_owned(), Bucket::default()));
            by_model.len() - 1
        });
    add(&mut by_model[m].1, r);
}

/// `agent:<name>` for a sidechain record (`agent:subagent` when unnamed), else `attributionSkill`,
/// else `main`. Empty strings are falsy.
///
/// The agent beats the skill: a subagent dispatched under a skill carries `attributionSkill`, and
/// keying on it would drop the call from the Agents column and fold it into the skill row (#70).
/// This deliberately diverges from `token_report.py`, which checks the skill first.
fn attribution_key(r: &crate::model::Record) -> String {
    if r.is_sidechain {
        let agent = r
            .attribution
            .agent
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or("subagent");
        return format!("agent:{agent}");
    }
    if let Some(skill) = r.attribution.skill.as_deref().filter(|s| !s.is_empty()) {
        return skill.to_owned();
    }
    "main".to_owned()
}

/// The bucket for `key`, appended in first-seen order when new.
fn slot<'a>(
    table: &'a mut Vec<(String, Bucket)>,
    ix: &mut HashMap<String, usize>,
    key: &str,
) -> &'a mut Bucket {
    let i = *ix.entry(key.to_owned()).or_insert_with(|| {
        table.push((key.to_owned(), Bucket::default()));
        table.len() - 1
    });
    &mut table[i].1
}

fn add(b: &mut Bucket, r: &crate::model::Record) {
    let u = &r.usage;
    b.input += u.input;
    b.cache_write_5m += u.cache_write_5m;
    b.cache_write_1h += u.cache_write_1h;
    b.cache_read += u.cache_read;
    b.output += u.output;
    b.thinking += u.thinking.unwrap_or(0);
    b.calls += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Harness, Record, RequestId, Usage};

    fn attributed(
        session: &str,
        branch: &str,
        basis: BranchBasis,
        output: u64,
    ) -> AttributedRecord {
        let usage = Usage {
            output,
            ..Usage::default()
        };
        AttributedRecord {
            record: Record::new(
                Harness::ClaudeCode,
                SessionId::new(session),
                RequestId::new("r"),
                usage,
            ),
            branch: branch.to_owned(),
            basis,
            subagent: None,
        }
    }

    fn un(calls: u64, tokens: u64) -> Unstamped {
        Unstamped { calls, tokens }
    }

    #[test]
    fn claim_mode_counts_only_none_records_no_session_claim_places() {
        let recs = [
            // Session a: only feat/x, so its (none) call is claimed there: placed.
            attributed("a", "feat/x", BranchBasis::Stamped, 1),
            attributed("a", "(none)", BranchBasis::Stamped, 100),
            // Session b: no work branch at all: unplaced.
            attributed("b", "main", BranchBasis::Stamped, 1),
            attributed("b", "(none)", BranchBasis::Stamped, 7),
            // Session c: two work branches, so no claim: unplaced.
            attributed("c", "feat/x", BranchBasis::Stamped, 1),
            attributed("c", "fix/y", BranchBasis::Stamped, 1),
            attributed("c", "(none)", BranchBasis::Stamped, 3),
            // Session d: only (none): unplaced.
            attributed("d", "(none)", BranchBasis::Stamped, 5),
            // Excluded: counted by `subagents.unattributed*`.
            attributed("d", "(none)", BranchBasis::SubagentUnlinked, 50),
            // Ambiguous but still on no branch: counted.
            attributed("e", "(none)", BranchBasis::TimelineAmbiguous, 60),
            // Ambiguous between real branches: has a branch, not unstamped.
            attributed("f", "feat/x", BranchBasis::TimelineAmbiguous, 70),
        ];
        assert_eq!(count_unstamped(&recs, Mode::Claim), un(4, 75));
        assert_eq!(count_unstamped(&[], Mode::Claim), Unstamped::default());
    }

    #[test]
    fn strict_and_timeline_have_no_claim_so_every_none_record_counts() {
        let recs = [
            attributed("a", "feat/x", BranchBasis::Stamped, 1),
            attributed("a", "(none)", BranchBasis::Stamped, 100),
            attributed("b", "(none)", BranchBasis::SubagentUnlinked, 50),
        ];
        assert_eq!(count_unstamped(&recs, Mode::Strict), un(1, 100));
        assert_eq!(count_unstamped(&recs, Mode::Timeline), un(1, 100));
    }

    fn keyed(sidechain: bool, agent: Option<&str>, skill: Option<&str>) -> Record {
        let mut r = Record::new(
            Harness::ClaudeCode,
            SessionId::new("s"),
            RequestId::new("r"),
            Usage::default(),
        );
        r.is_sidechain = sidechain;
        r.attribution.agent = agent.map(str::to_owned);
        r.attribution.skill = skill.map(str::to_owned);
        r
    }

    #[test]
    fn sidechain_with_skill_is_keyed_by_agent_not_skill() {
        assert_eq!(
            attribution_key(&keyed(true, Some("worker"), Some("review"))),
            "agent:worker"
        );
        assert_eq!(
            attribution_key(&keyed(true, None, Some("review"))),
            "agent:subagent"
        );
    }

    #[test]
    fn sidechain_without_skill_is_keyed_by_agent() {
        assert_eq!(
            attribution_key(&keyed(true, Some("worker"), None)),
            "agent:worker"
        );
    }

    #[test]
    fn main_with_skill_keeps_the_skill_key() {
        assert_eq!(
            attribution_key(&keyed(false, None, Some("review"))),
            "review"
        );
    }

    #[test]
    fn main_without_skill_is_main() {
        assert_eq!(attribution_key(&keyed(false, None, None)), "main");
    }

    #[test]
    fn sidechain_skill_records_reach_the_agents_set_and_keep_totals() {
        let mut a = attributed("s", "feat/x", BranchBasis::Stamped, 10);
        a.record.is_sidechain = true;
        a.record.attribution.agent = Some("worker".to_owned());
        a.record.attribution.skill = Some("review".to_owned());
        let sel = [SelectedRecord {
            attributed: a,
            kept_by: Selection::Exact,
        }];
        let s = summarize(
            "feat/x",
            Mode::Claim,
            &sel,
            (0, 0),
            HeuristicCount::default(),
        );
        let keys: Vec<_> = s.by_attribution.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["agent:worker"]);
        assert!(s.sessions[0].1.agents.contains("worker"));
        assert_eq!(s.sessions[0].1.tokens, 10);
    }
}
