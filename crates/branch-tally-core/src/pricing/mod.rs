//! Costs a summary from a [`PriceTable`].
//!
//! No price is defined in Rust: rates come from the table, which is resolved from
//! `.tally/prices.toml`, the per-user cache or the bundled `data/prices.toml` (see [`resolve`]).
//! Each layer is a complete table and the first found wins whole, so one table has one date.
//!
//! Arithmetic follows the reference's operation order so results are bit-identical: a token count
//! is converted to `f64` first (as Python does for `int * float`), `w5 * cin * 1.25` is
//! `(w5 * cin) * 1.25`, and the terms are added left to right. Totals use [`sum_costs`], which
//! reproduces the compensated summation of Python 3.12+'s `sum()` over floats.

mod litellm;
mod resolve;
mod table;

pub use litellm::{LitellmImport, from_litellm};
pub use resolve::{
    FeedError, FsText, PriceFeed, ResolveError, Resolved, TextSource, UpdateError, fetch_table,
    resolve,
};
pub use table::{ModelPrice, PriceTable, PriceTableError};

use crate::model::{Bucket, Costs, Summary};

/// Python's `int` to `float` conversion for a token count.
#[expect(
    clippy::cast_precision_loss,
    reason = "matches Python's int-to-float conversion, which rounds the same way"
)]
fn f(n: u64) -> f64 {
    n as f64
}

/// Estimated USD cost of `bucket` for `model` (`Bucket.cost`); `None` when the table has no
/// price for the model. Thinking is not priced: it is inside output.
#[must_use]
pub fn cost(bucket: &Bucket, model: &str, table: &PriceTable) -> Option<f64> {
    let entry = table.lookup(model)?;
    let cin = entry.input;
    let cout = entry.output;
    let m5 = entry.cache_write_5m.unwrap_or(table.cache_write_5m);
    let m1h = entry.cache_write_1h.unwrap_or(table.cache_write_1h);
    let read = entry.cache_read.unwrap_or(cin * table.cache_read_factor);
    let sum = f(bucket.input) * cin
        + f(bucket.cache_write_5m) * cin * m5
        + f(bucket.cache_write_1h) * cin * m1h
        + f(bucket.cache_read) * read
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

/// Costs every model of `summary` with `table`, keeping its order. The total sums in that same
/// first-seen order, as the reference's `as_json` does; unpriced models add 0 and are counted.
/// `price_table` records the table's date and origin.
#[must_use]
pub fn price(summary: &Summary, table: &PriceTable) -> Costs {
    let by_model: Vec<(String, Option<f64>)> = summary
        .by_model
        .iter()
        .map(|(model, bucket)| (model.clone(), cost(bucket, model, table)))
        .collect();
    let total = sum_costs(by_model.iter().map(|(_, c)| c.unwrap_or(0.0)));
    let unpriced_models = by_model.iter().filter(|(_, c)| c.is_none()).count() as u64;
    Costs {
        by_model,
        total,
        unpriced_models,
        price_table: table.info(),
    }
}
