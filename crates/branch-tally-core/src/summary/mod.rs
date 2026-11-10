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

use std::collections::HashMap;

use crate::model::{Bucket, Mode, Origin, SelectedRecord, SessionId, SessionRow, Summary};

/// The `origin` text used when a record has no origin. The reference always has one, so this can
/// only come from a source that could not tell; it is never counted as a worktree session.
const UNKNOWN_ORIGIN: &str = "unknown";

/// Turns the records selected for `branch` into a [`Summary`].
///
/// `mode` is carried through unchanged. Each record's assigned branch is compared with `branch`
/// to count off-branch tokens.
#[must_use]
pub fn summarize(branch: &str, mode: Mode, selected: &[SelectedRecord]) -> Summary {
    let mut by_model: Vec<(String, Bucket)> = Vec::new();
    let mut model_ix: HashMap<String, usize> = HashMap::new();
    let mut by_attribution: Vec<(String, Bucket)> = Vec::new();
    let mut attr_ix: HashMap<String, usize> = HashMap::new();
    let mut sessions: Vec<(SessionId, SessionRow)> = Vec::new();
    let mut session_ix: HashMap<SessionId, usize> = HashMap::new();
    // Parallel to `sessions`: whether the session's origin is known and not the main checkout.
    let mut is_worktree: Vec<bool> = Vec::new();
    let mut footnote = crate::model::Footnote::default();

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

        if r.is_sidechain {
            footnote.subagent_calls += 1;
        }
        if attributed.branch != branch {
            footnote.off_branch_tokens += u.tokens();
        }
    }
    footnote.worktree_sessions = is_worktree.iter().filter(|w| **w).count() as u64;

    let mut total = Bucket::default();
    for (_, b) in &by_model {
        total.input += b.input;
        total.cache_write_5m += b.cache_write_5m;
        total.cache_write_1h += b.cache_write_1h;
        total.cache_read += b.cache_read;
        total.output += b.output;
        total.thinking += b.thinking;
        total.calls += b.calls;
    }

    Summary {
        branch: branch.to_owned(),
        mode,
        total,
        footnote,
        by_model,
        by_attribution,
        sessions,
        subagents: crate::model::SubagentSummary::default(),
    }
}

/// `attributionSkill`, else `agent:<name>` for a sidechain, else `main`. Empty strings are falsy.
fn attribution_key(r: &crate::model::Record) -> String {
    if let Some(skill) = r.attribution.skill.as_deref().filter(|s| !s.is_empty()) {
        return skill.to_owned();
    }
    if r.is_sidechain {
        let agent = r
            .attribution
            .agent
            .as_deref()
            .filter(|a| !a.is_empty())
            .unwrap_or("subagent");
        return format!("agent:{agent}");
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
