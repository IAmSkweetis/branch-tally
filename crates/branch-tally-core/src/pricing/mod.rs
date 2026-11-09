//! Costs a summary.
//!
//! For the `report` port the rates are one const table copied verbatim from `PRICES` and
//! `CACHE_READ_RATE` in `reference/token_report.py`, to be replaced by data in issue #5. No other
//! module may define a price.
//!
//! Arithmetic follows the reference's operation order so results are bit-identical: a token count
//! is converted to `f64` first (as Python does for `int * float`), `w5 * cin * 1.25` is
//! `(w5 * cin) * 1.25`, and the terms are added left to right. Totals use [`sum_costs`], which
//! reproduces the compensated summation of Python 3.12+'s `sum()` over floats.

use crate::model::{Bucket, Costs, PriceTableInfo, Summary};

/// `$/MTok` list rates as `(input, output)`, verbatim from `PRICES` in the reference. Replaced by
/// data in #5.
const PRICES: &[(&str, (f64, f64))] = &[
    ("claude-fable-5-1", (10.0, 50.0)),
    ("claude-fable-5", (10.0, 50.0)),
    ("claude-opus-5", (5.0, 25.0)),
    ("claude-opus-4-8", (5.0, 25.0)),
    ("claude-opus-4-7", (5.0, 25.0)),
    ("claude-opus-4-6", (5.0, 25.0)),
    ("claude-sonnet-5", (2.0, 10.0)),
    ("claude-sonnet-4-6", (3.0, 15.0)),
    ("claude-haiku-4-5", (1.0, 5.0)),
];

/// Cache-read `$/MTok` where it is not 0.1x input, verbatim from `CACHE_READ_RATE`.
const CACHE_READ_RATE: &[(&str, f64)] = &[("claude-fable-5-1", 0.25)];

/// The longest key that `model` equals or starts with. An exact match is also the longest
/// prefix, so one pass covers the reference's "exact first, then longest prefix".
fn longest_match<T: Copy>(table: &[(&str, T)], model: &str) -> Option<T> {
    table
        .iter()
        .filter(|(known, _)| model.starts_with(known))
        .max_by_key(|(known, _)| known.len())
        .map(|(_, v)| *v)
}

/// Rates for `model` as `(input, output)` in dollars per million tokens, or `None` when the
/// model is unknown.
#[must_use]
pub fn price_for(model: &str) -> Option<(f64, f64)> {
    longest_match(PRICES, model)
}

/// Cache-read rate in dollars per million tokens for `model`; defaults to `0.1 x input_rate`.
#[must_use]
pub fn cache_read_rate(model: &str, input_rate: f64) -> f64 {
    longest_match(CACHE_READ_RATE, model).unwrap_or(input_rate * 0.1)
}

/// Python's `int` to `float` conversion for a token count.
#[expect(
    clippy::cast_precision_loss,
    reason = "matches Python's int-to-float conversion, which rounds the same way"
)]
fn f(n: u64) -> f64 {
    n as f64
}

/// Estimated USD cost of `bucket` for `model` (`Bucket.cost`); `None` when the model has no
/// price. Thinking is not priced: it is inside output.
#[must_use]
pub fn cost(bucket: &Bucket, model: &str) -> Option<f64> {
    let (cin, cout) = price_for(model)?;
    let sum = f(bucket.input) * cin
        + f(bucket.cache_write_5m) * cin * 1.25
        + f(bucket.cache_write_1h) * cin * 2.0
        + f(bucket.cache_read) * cache_read_rate(model, cin)
        + f(bucket.output) * cout;
    Some(sum / 1_000_000.0)
}

/// Python 3.12+ `sum()` over floats: Neumaier compensated summation, as the interpreter does it.
#[must_use]
pub fn sum_costs(costs: impl IntoIterator<Item = f64>) -> f64 {
    let mut total = 0.0_f64;
    let mut comp = 0.0_f64;
    for x in costs {
        let t = total + x;
        if total.abs() >= x.abs() {
            comp += (total - t) + x;
        } else {
            comp += (x - t) + total;
        }
        total = t;
    }
    if comp != 0.0 && comp.is_finite() {
        total += comp;
    }
    total
}

/// Costs every model of `summary`, keeping its order. The total sums in that same first-seen
/// order, as the reference's `as_json` does; unpriced models add 0 and are counted.
#[must_use]
pub fn price(summary: &Summary) -> Costs {
    let by_model: Vec<(String, Option<f64>)> = summary
        .by_model
        .iter()
        .map(|(model, bucket)| (model.clone(), cost(bucket, model)))
        .collect();
    let total = sum_costs(by_model.iter().map(|(_, c)| c.unwrap_or(0.0)));
    let unpriced_models = by_model.iter().filter(|(_, c)| c.is_none()).count() as u64;
    Costs {
        by_model,
        total,
        unpriced_models,
        price_table: PriceTableInfo::default(),
    }
}
