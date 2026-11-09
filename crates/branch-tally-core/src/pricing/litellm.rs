//! Conversion of `LiteLLM`'s `model_prices_and_context_window.json` into a [`PriceTable`].
//!
//! Which entries are kept: key starts with `claude-`, `litellm_provider` is `anthropic` and `mode`
//! is `chat` (or absent). That deliberately excludes provider-prefixed duplicates
//! (`anthropic/claude-...`), Bedrock and Vertex ids (`anthropic.claude-...`, `vertex_ai/...`): the
//! transcripts name models by their Anthropic id, and a cloud reseller's rate would be a guess.
//! Other providers' models and non-model entries (`sample_spec`) are not Claude and are ignored.
//!
//! Rates are `$/token` in the feed and `$/MTok` here, rounded to nine decimals so `5e-6` becomes
//! exactly `5.0` rather than `5.000000000000001`. Only the base rates are read; `LiteLLM`'s
//! long-context tiers (`*_above_200k_tokens`) are ignored.
//!
//! Cache fields are checked against the table's global multipliers rather than trusted:
//! `cache_read_input_token_cost` that differs from 0.1 x input becomes a per-model `cache_read`;
//! `cache_creation_input_token_cost` and `..._above_1hr` that differ from 1.25 x and 2 x input
//! become per-model `cache_write_5m` / `cache_write_1h` multipliers. Nothing is silently globalised.

use serde_json::Value;

use crate::model::PriceTableSource;

use super::table::{ModelPrice, PriceTable, PriceTableError, valid_date};

const TOLERANCE: f64 = 1e-9;

/// The result of converting a `LiteLLM` feed.
#[derive(Debug, Clone, PartialEq)]
pub struct LitellmImport {
    /// The converted table, tagged as a cache table.
    pub table: PriceTable,
    /// Ids of Anthropic `claude-*` chat entries left out because they lack a usable input or
    /// output cost. Counted so the CLI can say so.
    pub skipped: Vec<String>,
}

/// Converts `LiteLLM` JSON to a table dated `as_of`.
///
/// # Errors
///
/// [`PriceTableError`] when the JSON is not an object, or `as_of` is not a valid date.
pub fn from_litellm(json: &str, as_of: &str) -> Result<LitellmImport, PriceTableError> {
    if !valid_date(as_of) {
        return Err(PriceTableError::InvalidAsOf);
    }
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(json) else {
        return Err(PriceTableError::BadFeed);
    };
    let mut models = Vec::new();
    let mut skipped = Vec::new();
    for (id, entry) in &root {
        if !id.starts_with("claude-") || !is_anthropic_chat(entry) {
            continue;
        }
        match convert(id, entry) {
            Some(m) => models.push(m),
            None => skipped.push(id.clone()),
        }
    }
    skipped.sort();
    Ok(LitellmImport {
        table: PriceTable::from_models(as_of.to_owned(), PriceTableSource::Cache, models),
        skipped,
    })
}

fn is_anthropic_chat(entry: &Value) -> bool {
    entry.get("litellm_provider").and_then(Value::as_str) == Some("anthropic")
        && entry.get("mode").is_none_or(|m| m.as_str() == Some("chat"))
}

/// Per-token cost field as `$/MTok`.
fn mtok(entry: &Value, key: &str) -> Option<f64> {
    let v = entry.get(key)?.as_f64()?;
    (v.is_finite() && v >= 0.0).then(|| round9(v * 1_000_000.0))
}

fn round9(x: f64) -> f64 {
    (x * 1e9).round() / 1e9
}

fn convert(id: &str, entry: &Value) -> Option<ModelPrice> {
    let input = mtok(entry, "input_cost_per_token")?;
    let output = mtok(entry, "output_cost_per_token")?;
    let cache_read =
        mtok(entry, "cache_read_input_token_cost").filter(|r| (r - input * 0.1).abs() > TOLERANCE);
    let multiplier = |key: &str, global: f64| -> Option<f64> {
        if input <= 0.0 {
            return None;
        }
        let m = round9(mtok(entry, key)? / input);
        ((m - global).abs() > TOLERANCE).then_some(m)
    };
    Some(ModelPrice {
        id: id.to_owned(),
        input,
        output,
        cache_read,
        cache_write_5m: multiplier("cache_creation_input_token_cost", 1.25),
        cache_write_1h: multiplier("cache_creation_input_token_cost_above_1hr", 2.0),
    })
}
