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
        }
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
