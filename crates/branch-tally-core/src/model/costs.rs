//! What `pricing` produces and `render` consumes. Data only: nothing here is rendered by the
//! types themselves.

use serde::{Deserialize, Serialize};

/// Estimated cost per model plus the total (`Bucket.cost` and the `total_cost` sum in `render`
/// and `as_json` of `reference/token_report.py`).
///
/// `by_model` is in the same first-seen order as `Summary::by_model`. The reference sums
/// `cost or 0.0`, so an unknown model contributes 0 to `total`; `unpriced_models` counts those so
/// a later stage can label the total as a floor (Governance II). The port does not render that.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Costs {
    /// Cost in USD per model id; `None` for a model with no known price (rendered as an em dash
    /// or JSON `null`).
    pub by_model: Vec<(String, Option<f64>)>,
    /// Sum of the known costs, unpriced models counting as 0.
    pub total: f64,
    /// How many models in `by_model` have no price, so `total` is a floor when this is non-zero.
    pub unpriced_models: u64,
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
}
