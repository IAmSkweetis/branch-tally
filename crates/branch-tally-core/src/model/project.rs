//! The project-wide report: one row per ledger recording, deduplicated totals, the overlap between
//! branches and the cumulative cost series.
//!
//! Data and flag helpers only. The `project` module fills these in from the ledger and `render`
//! formats them; neither re-derives a number from the other.
//!
//! Rows are keyed by the ledger's `recorded_for`. A request claimed by two branches is stored once
//! per branch, so rows may sum to more than [`ProjectTotals`]: the totals count each `request_id`
//! once and [`SharedOverlap`] says how much was counted twice. A floor anywhere is a floor
//! everywhere (Governance II). Only branch names, dates and numbers are held (Governance III).

use serde::{Deserialize, Serialize};

use super::{Bucket, HeuristicBreakdown, RowCost, Timestamp};

/// One recording (`recorded_for`) in the project: its own totals, uncorrected for overlap.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BranchRow {
    /// The ledger's `recorded_for` value.
    pub branch: String,
    /// The latest `ts` among the branch's ledger lines, ISO-8601 as recorded. `None` when no line
    /// carried a timestamp; such a branch has no place on the time axis and is counted in
    /// [`ProjectReport::undated_branches`].
    pub last_call: Option<Timestamp>,
    /// Calls and tokens for this row, shared calls included.
    pub usage: Bucket,
    /// Estimated cost; [`RowCost::is_floor`] when some model had no price.
    pub cost: RowCost,
    /// The model ids with no known price, so a renderer can name them.
    pub unpriced_models: Vec<String>,
    /// Calls whose branch was not certain, by heuristic.
    pub uncertain: HeuristicBreakdown,
    /// Calls with a missing or unparsable `ts`. They are in `usage` and `cost` but cannot move
    /// `last_call`, so when this is non-zero `last_call` may be earlier than the real last call.
    #[serde(default)]
    pub undated_calls: u64,
    /// `true` when a token or call total behind this figure saturated at `u64::MAX` (see
    /// [`Footnote::totals_overflowed`](super::Footnote)): the figures are a floor and
    /// unreliable. Omitted from JSON when `false`; absent in older serialized data.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub totals_overflowed: bool,
}

impl BranchRow {
    /// Whether this row's cost is only a lower bound: an unpriced model, or a saturated total.
    #[must_use]
    pub fn is_floor(&self) -> bool {
        self.totals_overflowed || self.cost.is_floor() || !self.unpriced_models.is_empty()
    }
}

/// What was claimed by more than one branch: counted in each row, once in the totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedOverlap {
    /// Calls (distinct request ids) present under two or more branches.
    pub calls: u64,
    /// Tokens in those calls, counted once.
    pub tokens: u64,
    /// Their cost, counted once. A floor when `cost.is_floor()`.
    pub cost: RowCost,
}

/// Project totals with every `request_id` counted once.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectTotals {
    /// Deduplicated calls and tokens.
    pub usage: Bucket,
    /// Deduplicated cost; a floor when any contributing model was unpriced.
    pub cost: RowCost,
    /// Every model id with no known price, across all rows.
    pub unpriced_models: Vec<String>,
    /// Uncertain calls, counted once per request id.
    pub uncertain: HeuristicBreakdown,
    /// What the rows count more than once.
    pub overlap: SharedOverlap,
    /// Calls with a missing or unparsable `ts`, counted once per request id.
    #[serde(default)]
    pub undated_calls: u64,
    /// `true` when a token or call total behind the project total, or summed across branches, saturated at `u64::MAX` (see
    /// [`Footnote::totals_overflowed`](super::Footnote)): the figures are a floor and
    /// unreliable. Omitted from JSON when `false`; absent in older serialized data.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub totals_overflowed: bool,
}

impl ProjectTotals {
    /// Whether the project total is only a lower bound ("at least $X"): an unpriced model, or a
    /// saturated total.
    #[must_use]
    pub fn is_floor(&self) -> bool {
        self.totals_overflowed || self.cost.is_floor() || !self.unpriced_models.is_empty()
    }
}

/// One point of the cumulative cost-over-time series: one per dated branch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeriesPoint {
    /// The branch's last-call timestamp text, taken from a validated [`Timestamp`].
    pub date: String,
    /// The branch (`recorded_for`) this point adds.
    pub branch: String,
    /// Cost summed over this branch and every earlier one, in date order.
    pub cumulative_cost: f64,
    /// Whether `cumulative_cost` is only a floor: this or any earlier branch had unpriced models.
    pub is_floor: bool,
}

/// Everything the project report shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectReport {
    /// One row per `recorded_for`, in the order the aggregator chose.
    pub rows: Vec<BranchRow>,
    /// Deduplicated totals.
    pub totals: ProjectTotals,
    /// The cumulative series, ordered by `date`.
    pub series: Vec<SeriesPoint>,
    /// Rows with no timestamp, left off the series and counted here.
    pub undated_branches: u64,
}

impl ProjectReport {
    /// Whether any total in the report saturated: the project totals or any row. A renderer says
    /// the totals are unreliable floors when this is true.
    #[must_use]
    pub fn is_overflowed(&self) -> bool {
        self.totals.totals_overflowed || self.rows.iter().any(|r| r.totals_overflowed)
    }

    /// Whether any figure in the report is only a floor: the totals, any row or any series point.
    /// A renderer labels the project total "at least" when this is true.
    #[must_use]
    pub fn is_floor(&self) -> bool {
        self.totals.is_floor()
            || self.rows.iter().any(BranchRow::is_floor)
            || self.series.iter().any(|p| p.is_floor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn priced() -> RowCost {
        RowCost {
            known: 1.0,
            priced_models: 1,
            unpriced_models: 0,
        }
    }

    fn floor() -> RowCost {
        RowCost {
            known: 1.0,
            priced_models: 1,
            unpriced_models: 1,
        }
    }

    #[test]
    fn row_with_unpriced_models_is_a_floor() {
        let mut row = BranchRow {
            cost: priced(),
            ..BranchRow::default()
        };
        assert!(!row.is_floor());
        row.cost = floor();
        assert!(row.is_floor());
        row.cost = priced();
        row.unpriced_models.push("m".to_owned());
        assert!(row.is_floor());
    }

    #[test]
    fn totals_floor_follows_cost_or_named_models() {
        let mut t = ProjectTotals {
            cost: priced(),
            ..ProjectTotals::default()
        };
        assert!(!t.is_floor());
        t.unpriced_models.push("m".to_owned());
        assert!(t.is_floor());
    }

    #[test]
    fn report_is_a_floor_when_any_row_is_even_if_totals_look_exact() {
        let mut report = ProjectReport {
            rows: vec![BranchRow {
                cost: priced(),
                ..BranchRow::default()
            }],
            totals: ProjectTotals {
                cost: priced(),
                ..ProjectTotals::default()
            },
            ..ProjectReport::default()
        };
        assert!(!report.is_floor());
        report.rows.push(BranchRow {
            cost: floor(),
            ..BranchRow::default()
        });
        assert!(report.is_floor());
    }

    #[test]
    fn report_is_a_floor_when_a_series_point_is() {
        let report = ProjectReport {
            series: vec![SeriesPoint {
                is_floor: true,
                ..SeriesPoint::default()
            }],
            ..ProjectReport::default()
        };
        assert!(report.is_floor());
    }

    #[test]
    fn overflowed_row_and_totals_are_floors() {
        let row = BranchRow {
            cost: priced(),
            totals_overflowed: true,
            ..BranchRow::default()
        };
        assert!(row.is_floor());
        let totals = ProjectTotals {
            cost: priced(),
            totals_overflowed: true,
            ..ProjectTotals::default()
        };
        assert!(totals.is_floor());
    }

    #[test]
    fn report_overflow_follows_totals_or_any_row() {
        let mut report = ProjectReport::default();
        assert!(!report.is_overflowed());
        report.rows.push(BranchRow {
            totals_overflowed: true,
            ..BranchRow::default()
        });
        assert!(report.is_overflowed());
        assert!(report.is_floor());
        let only_totals = ProjectReport {
            totals: ProjectTotals {
                totals_overflowed: true,
                ..ProjectTotals::default()
            },
            ..ProjectReport::default()
        };
        assert!(only_totals.is_overflowed());
    }

    #[test]
    fn overflow_flag_is_omitted_from_json_when_false_and_defaults_when_absent() {
        let row = BranchRow::default();
        assert!(
            !serde_json::to_string(&row)
                .unwrap()
                .contains("totals_overflowed")
        );
        let json = serde_json::to_string(&BranchRow {
            totals_overflowed: true,
            ..BranchRow::default()
        })
        .unwrap();
        assert!(json.contains("\"totals_overflowed\":true"));
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("totals_overflowed");
        let back: BranchRow = serde_json::from_value(v).unwrap();
        assert!(!back.totals_overflowed);
    }

    #[test]
    fn report_round_trips_through_json() {
        let report = ProjectReport {
            rows: vec![BranchRow {
                branch: "feat/x".to_owned(),
                last_call: "2026-01-02T03:04:05Z".parse().ok(),
                cost: floor(),
                ..BranchRow::default()
            }],
            undated_branches: 2,
            ..ProjectReport::default()
        };
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(
            serde_json::from_str::<ProjectReport>(&json).unwrap(),
            report
        );
    }
}
