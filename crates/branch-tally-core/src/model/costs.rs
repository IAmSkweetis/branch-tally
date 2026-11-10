//! What `pricing` produces and `render` consumes. Data only: nothing here is rendered by the
//! types themselves.

use serde::{Deserialize, Serialize};

/// Estimated cost per model plus the total (`Bucket.cost` and the `total_cost` sum in `render`
/// and `as_json` of `reference/token_report.py`).
///
/// `by_model` is in the same first-seen order as `Summary::by_model`. The reference sums
/// `cost or 0.0`, so an unknown model contributes 0 to `total`; `unpriced_models` counts those so
/// a later stage can label the total as a floor (Governance II). `render` states that floor and the
/// date of `price_table`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Costs {
    /// Cost in USD per model id; `None` for a model with no known price (rendered as an em dash
    /// or JSON `null`).
    pub by_model: Vec<(String, Option<f64>)>,
    /// Sum of the known costs, unpriced models counting as 0.
    pub total: f64,
    /// How many models in `by_model` have no price, so `total` is a floor when this is non-zero.
    pub unpriced_models: u64,
    /// Which price table produced these numbers, so a report can state its date.
    pub price_table: PriceTableInfo,
    /// Estimated cost of each subagent breakdown row, index-aligned with
    /// `Summary::subagents.rows`. Empty when pricing has not filled it (or on old data), in which
    /// case a row has no cost to show; use [`Costs::subagent_row`].
    #[serde(default)]
    pub subagent_rows: Vec<RowCost>,
    /// The subagent section's total: the sum of the rows' known costs and model counts.
    #[serde(default)]
    pub subagent_total: RowCost,
}

/// A cost that may be only a floor: the sum of the priced parts and how many parts had no price.
///
/// Used per subagent row (parts are the row's models) and for the subagent section's total (the
/// rows' counts summed). Render shows an em dash when nothing was priced, and "at least $X" when
/// some models were priced and some not (Governance II).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RowCost {
    /// Sum of the known costs in USD, unpriced models counting as 0.
    pub known: f64,
    /// How many contributing models have a price.
    pub priced_models: u64,
    /// How many contributing models have no price.
    pub unpriced_models: u64,
}

impl RowCost {
    /// Whether `known` is only a lower bound because some model had no price.
    #[must_use]
    pub fn is_floor(&self) -> bool {
        self.unpriced_models > 0
    }

    /// Whether no model contributed a price, so there is no number to show. A row with no models
    /// at all counts as unpriced.
    #[must_use]
    pub fn is_unpriced(&self) -> bool {
        self.priced_models == 0
    }
}

/// Provenance of the price table behind a [`Costs`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceTableInfo {
    /// The table's date, ISO-8601 `YYYY-MM-DD`. Empty when no table was consulted.
    pub as_of: String,
    /// Where the table came from.
    pub source: PriceTableSource,
}

/// Which layer of the lookup order supplied the price table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PriceTableSource {
    /// The table compiled into the binary.
    #[default]
    Bundled,
    /// The refreshed user cache written by `prices update`.
    Cache,
    /// The repository's `.tally/prices.toml`.
    Override,
}

impl Costs {
    /// The cost recorded for `model`, or `None` when the model is absent or unpriced.
    #[must_use]
    pub fn cost_for(&self, model: &str) -> Option<f64> {
        self.by_model
            .iter()
            .find(|(m, _)| m == model)
            .and_then(|(_, c)| *c)
    }

    /// The cost of subagent breakdown row `index` (aligned with `Summary::subagents.rows`), or
    /// `None` when pricing recorded nothing for it.
    #[must_use]
    pub fn subagent_row(&self, index: usize) -> Option<&RowCost> {
        self.subagent_rows.get(index)
    }

    /// Whether `total` is only a lower bound because some model had no price.
    #[must_use]
    pub fn is_floor(&self) -> bool {
        self.unpriced_models > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Costs {
        Costs {
            by_model: vec![("a".to_owned(), Some(1.5)), ("b".to_owned(), None)],
            total: 1.5,
            unpriced_models: 1,
            price_table: PriceTableInfo::default(),
            subagent_rows: vec![RowCost {
                known: 0.5,
                priced_models: 1,
                unpriced_models: 1,
            }],
            subagent_total: RowCost::default(),
        }
    }

    #[test]
    fn row_cost_floor_unpriced_alignment_and_old_data() {
        let c = sample();
        let r = c.subagent_row(0).unwrap();
        assert!(r.is_floor() && !r.is_unpriced());
        assert!(c.subagent_row(1).is_none());
        assert!(RowCost::default().is_unpriced() && !RowCost::default().is_floor());
        let old = r#"{"by_model":[],"total":0.0,"unpriced_models":0,"price_table":{"as_of":"","source":"bundled"}}"#;
        let back: Costs = serde_json::from_str(old).unwrap();
        assert_eq!(back.subagent_rows.len(), 0);
    }

    #[test]
    fn cost_for_distinguishes_priced_unpriced_and_absent() {
        let c = sample();
        assert_eq!(c.cost_for("a"), Some(1.5));
        assert_eq!(c.cost_for("b"), None);
        assert_eq!(c.cost_for("zzz"), None);
    }

    #[test]
    fn total_is_a_floor_only_when_a_model_is_unpriced() {
        assert!(sample().is_floor());
        assert!(!Costs::default().is_floor());
    }

    #[test]
    fn costs_round_trip_with_null_for_unpriced() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("null"));
        assert_eq!(serde_json::from_str::<Costs>(&json).unwrap(), sample());
    }

    #[test]
    fn price_table_provenance_serializes_source_lowercase() {
        let info = PriceTableInfo {
            as_of: "2026-11-09".to_owned(),
            source: PriceTableSource::Override,
        };
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["as_of"], "2026-11-09");
        assert_eq!(v["source"], "override");
        assert_eq!(
            serde_json::to_value(PriceTableSource::Cache).unwrap(),
            "cache"
        );
        assert_eq!(
            serde_json::to_value(PriceTableSource::Bundled).unwrap(),
            "bundled"
        );
    }
}
