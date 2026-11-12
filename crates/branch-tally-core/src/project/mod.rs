//! Project-wide totals and the cost-over-time series, built from the ledger.
//!
//! The shared data types live in [`crate::model`] (`ProjectReport` and friends). [`build`] reuses
//! the single-branch pipeline for every number (`summarize` then `price`, with the same
//! ambiguity count `report::run_from_ledger` uses on a ledger-only read), so a row matches what
//! `report <branch> --from-ledger` shows for that branch.
//!
//! Rows are keyed by `recorded_for`. A request claimed by two branches is stored once per branch:
//! rows keep their own totals, the project total counts each request id once (the first stored
//! copy) and the overlap says what was counted twice. The series adds each request at the
//! earliest dated branch that claims it, so its last point equals the total over dated branches.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::ledger::{Ledger, LedgerEntry};
use crate::model::{
    BranchRow, Costs, HeuristicBreakdown, Mode, ProjectReport, ProjectTotals, RowCost, SeriesPoint,
    SharedOverlap, Summary,
};
use crate::pricing::{PriceTable, price, sum_costs};

/// Summary and cost of a set of ledger entries, built as `run_from_ledger` builds a ledger-only
/// report: every entry is "selected" and the ambiguity count runs over those same entries.
struct Priced {
    summary: Summary,
    costs: Costs,
}

impl Priced {
    fn of(
        label: &str,
        entries: &[&LedgerEntry],
        main: Option<&Path>,
        mode: Mode,
        table: &PriceTable,
    ) -> Self {
        let selected: Vec<_> = entries.iter().map(|e| e.to_selected(main)).collect();
        let attributed: Vec<_> = selected.iter().map(|s| s.attributed.clone()).collect();
        let ambiguous = crate::summary::count_ambiguous(&attributed, &selected);
        let summary = crate::summary::summarize(label, mode, &selected, (0, 0), ambiguous);
        let costs = price(&summary, table);
        Self { summary, costs }
    }

    fn row_cost(&self) -> RowCost {
        let priced = self
            .costs
            .by_model
            .iter()
            .filter(|(_, c)| c.is_some())
            .count() as u64;
        RowCost {
            known: self.costs.total,
            priced_models: priced,
            unpriced_models: self.costs.unpriced_models,
        }
    }

    fn unpriced_names(&self) -> Vec<String> {
        self.costs
            .by_model
            .iter()
            .filter(|(_, c)| c.is_none())
            .map(|(m, _)| m.clone())
            .collect()
    }

    fn uncertain(&self) -> HeuristicBreakdown {
        self.summary.footnote.heuristics
    }
}

/// Builds the project report from a read ledger. Pure: nothing here touches git, the clock or
/// the filesystem.
///
/// `main` is the main checkout, used to restore worktree origins as `run_from_ledger` does.
/// `mode` is the mode the single-branch report would use (strict leaves the heuristic
/// breakdown empty, as it does there).
///
/// Rows are ordered by `last_call` ascending (ISO-8601 strings compared as text), ties by branch
/// name, undated rows last. Branches with no timestamp stay in `rows`, are left off the series
/// and are counted in `undated_branches`.
#[must_use]
pub fn build(
    ledger: &Ledger,
    main: Option<&Path>,
    mode: Mode,
    table: &PriceTable,
) -> ProjectReport {
    let mut groups: Vec<(&str, Vec<&LedgerEntry>)> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for entry in ledger.entries() {
        let i = *index.entry(entry.recorded_for.as_str()).or_insert_with(|| {
            groups.push((entry.recorded_for.as_str(), Vec::new()));
            groups.len() - 1
        });
        groups[i].1.push(entry);
    }

    let mut rows: Vec<(BranchRow, Vec<&LedgerEntry>)> = groups
        .into_iter()
        .map(|(branch, entries)| {
            let priced = Priced::of(branch, &entries, main, mode, table);
            let last_call = entries
                .iter()
                .filter_map(|e| e.ts.as_deref())
                .filter(|ts| !ts.is_empty())
                .max()
                .map(str::to_owned);
            let row = BranchRow {
                branch: branch.to_owned(),
                last_call,
                usage: priced.summary.total,
                cost: priced.row_cost(),
                unpriced_models: priced.unpriced_names(),
                uncertain: priced.uncertain(),
            };
            (row, entries)
        })
        .collect();
    rows.sort_by(|(a, _), (b, _)| {
        (a.last_call.is_none(), &a.last_call, &a.branch).cmp(&(
            b.last_call.is_none(),
            &b.last_call,
            &b.branch,
        ))
    });

    // Each request id counted once, at its first stored copy.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut deduped: Vec<&LedgerEntry> = Vec::new();
    let mut claims: HashMap<&str, u64> = HashMap::new();
    for entry in ledger.entries() {
        *claims.entry(entry.request_id.as_str()).or_insert(0) += 1;
        if seen.insert(entry.request_id.as_str()) {
            deduped.push(entry);
        }
    }
    let shared: Vec<&LedgerEntry> = deduped
        .iter()
        .copied()
        .filter(|e| claims.get(e.request_id.as_str()).copied().unwrap_or(0) > 1)
        .collect();
    let total = Priced::of("project", &deduped, main, mode, table);
    let overlap = Priced::of("shared", &shared, main, mode, table);

    let totals = ProjectTotals {
        usage: total.summary.total,
        cost: total.row_cost(),
        unpriced_models: total.unpriced_names(),
        uncertain: total.uncertain(),
        overlap: SharedOverlap {
            calls: overlap.summary.total.calls,
            tokens: overlap.summary.total.tokens(),
            cost: overlap.row_cost(),
        },
    };

    let mut series = Vec::new();
    let mut counted: HashSet<&str> = HashSet::new();
    let mut cumulative = 0.0_f64;
    let mut floor = false;
    for (row, entries) in &rows {
        let Some(date) = &row.last_call else { continue };
        let fresh: Vec<&LedgerEntry> = entries
            .iter()
            .copied()
            .filter(|e| counted.insert(e.request_id.as_str()))
            .collect();
        let added = Priced::of(&row.branch, &fresh, main, mode, table);
        cumulative = sum_costs([cumulative, added.costs.total]);
        floor = floor || row.is_floor();
        series.push(SeriesPoint {
            date: date.clone(),
            branch: row.branch.clone(),
            cumulative_cost: cumulative,
            is_floor: floor,
        });
    }

    let undated_branches = rows.iter().filter(|(r, _)| r.last_call.is_none()).count() as u64;
    ProjectReport {
        rows: rows.into_iter().map(|(r, _)| r).collect(),
        totals,
        series,
        undated_branches,
    }
}
