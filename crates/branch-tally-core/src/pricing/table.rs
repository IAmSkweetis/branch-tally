//! The price table: its types, TOML schema, parsing and deterministic serialization.
//!
//! ```toml
//! as_of = "2026-11-09"            # required, quoted ISO-8601 date
//!
//! [cache_write]                   # optional global multipliers on the input rate
//! five_minute = 1.25
//! one_hour = 2.0
//!
//! [cache_read]                    # optional
//! default_factor = 0.1            # cache reads cost this fraction of the input rate
//!
//! [models."claude-opus-5"]        # one table per model id, matched by longest prefix
//! input = 5.0                     # $/MTok, required
//! output = 25.0                   # $/MTok, required
//! cache_read = 0.5                # optional $/MTok override of the default factor
//! cache_write_5m = 1.25           # optional per-model multiplier overriding the global one
//! cache_write_1h = 2.0            # optional likewise
//! ```
//!
//! Unknown keys are an error, so a typo cannot silently price with a default. Rates must be finite
//! and non-negative. The text never carries transcript content, and error messages never quote it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;

use crate::model::{PriceTableInfo, PriceTableSource};

/// Why a price table could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PriceTableError {
    /// The text is not valid TOML for this schema. Carries the parser's message, never the text.
    #[error("invalid price table: {0}")]
    Invalid(String),
    /// `as_of` is absent.
    #[error("price table has no `as_of` date")]
    MissingAsOf,
    /// `as_of` is not a real `YYYY-MM-DD` date.
    #[error("price table `as_of` is not a valid YYYY-MM-DD date")]
    InvalidAsOf,
    /// A rate or multiplier is negative, infinite or NaN, or a model id is empty.
    #[error("price table has an invalid rate or model id for `{0}`")]
    InvalidRate(String),
    /// The `LiteLLM` feed is not the expected JSON object.
    #[error("price feed is not a JSON object of models")]
    BadFeed,
}

/// Rates for one model, in dollars per million tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrice {
    /// Model id, matched against transcript ids by longest prefix.
    pub id: String,
    /// Input rate.
    pub input: f64,
    /// Output rate.
    pub output: f64,
    /// Cache-read rate when it is not `default_factor x input`.
    pub cache_read: Option<f64>,
    /// 5-minute cache-write multiplier when it differs from the table's.
    pub cache_write_5m: Option<f64>,
    /// 1-hour cache-write multiplier when it differs from the table's.
    pub cache_write_1h: Option<f64>,
}

/// A complete price table with one date and one origin.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceTable {
    pub(crate) as_of: String,
    pub(crate) source: PriceTableSource,
    pub(crate) cache_write_5m: f64,
    pub(crate) cache_write_1h: f64,
    pub(crate) cache_read_factor: f64,
    pub(crate) models: Vec<ModelPrice>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    as_of: Option<String>,
    cache_write: Option<RawCacheWrite>,
    cache_read: Option<RawCacheRead>,
    #[serde(default)]
    models: BTreeMap<String, RawModel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCacheWrite {
    five_minute: Option<f64>,
    one_hour: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCacheRead {
    default_factor: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModel {
    input: f64,
    output: f64,
    cache_read: Option<f64>,
    cache_write_5m: Option<f64>,
    cache_write_1h: Option<f64>,
}

impl PriceTable {
    /// The table compiled into the binary. If the embedded file ever failed to parse (a test pins
    /// that it does not), the result is an empty table that prices nothing, so every model is
    /// counted as unpriced rather than priced wrongly.
    #[must_use]
    pub fn bundled() -> &'static PriceTable {
        static BUNDLED: std::sync::OnceLock<PriceTable> = std::sync::OnceLock::new();
        BUNDLED.get_or_init(|| {
            Self::parse(
                include_str!("../../data/prices.toml"),
                PriceTableSource::Bundled,
            )
            .unwrap_or_else(|_| PriceTable {
                as_of: String::new(),
                source: PriceTableSource::Bundled,
                cache_write_5m: DEFAULT_W5,
                cache_write_1h: DEFAULT_W1H,
                cache_read_factor: DEFAULT_READ_FACTOR,
                models: Vec::new(),
            })
        })
    }

    /// Parses `text` and tags the table with where it came from.
    ///
    /// # Errors
    ///
    /// [`PriceTableError`] for malformed TOML, unknown keys, a missing or invalid `as_of`, or an
    /// invalid rate.
    pub fn parse(text: &str, source: PriceTableSource) -> Result<Self, PriceTableError> {
        let raw: Raw =
            toml::from_str(text).map_err(|e| PriceTableError::Invalid(e.message().to_owned()))?;
        let as_of = raw.as_of.ok_or(PriceTableError::MissingAsOf)?;
        if !valid_date(&as_of) {
            return Err(PriceTableError::InvalidAsOf);
        }
        let cw = raw.cache_write.unwrap_or(RawCacheWrite {
            five_minute: None,
            one_hour: None,
        });
        let five_min = cw.five_minute.unwrap_or(DEFAULT_W5);
        let one_hour = cw.one_hour.unwrap_or(DEFAULT_W1H);
        let cache_read_factor = raw
            .cache_read
            .and_then(|c| c.default_factor)
            .unwrap_or(DEFAULT_READ_FACTOR);
        check(
            "cache multipliers",
            &[five_min, one_hour, cache_read_factor],
        )?;
        let mut models = Vec::with_capacity(raw.models.len());
        for (id, m) in raw.models {
            let optional = [m.cache_read, m.cache_write_5m, m.cache_write_1h];
            if id.is_empty() {
                return Err(PriceTableError::InvalidRate("<empty id>".to_owned()));
            }
            check(&id, &[m.input, m.output])?;
            check(&id, &optional.into_iter().flatten().collect::<Vec<_>>())?;
            models.push(ModelPrice {
                id,
                input: m.input,
                output: m.output,
                cache_read: m.cache_read,
                cache_write_5m: m.cache_write_5m,
                cache_write_1h: m.cache_write_1h,
            });
        }
        Ok(Self {
            as_of,
            source,
            cache_write_5m: five_min,
            cache_write_1h: one_hour,
            cache_read_factor,
            models,
        })
    }

    /// A table with the default multipliers from already-validated parts; `models` are sorted.
    pub(crate) fn from_models(
        as_of: String,
        source: PriceTableSource,
        mut models: Vec<ModelPrice>,
    ) -> Self {
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            as_of,
            source,
            cache_write_5m: DEFAULT_W5,
            cache_write_1h: DEFAULT_W1H,
            cache_read_factor: DEFAULT_READ_FACTOR,
            models,
        }
    }

    /// The table's global multipliers as `(five_minute, one_hour, cache_read_factor)`.
    #[must_use]
    pub fn multipliers(&self) -> (f64, f64, f64) {
        (
            self.cache_write_5m,
            self.cache_write_1h,
            self.cache_read_factor,
        )
    }

    /// The table's date, `YYYY-MM-DD`.
    #[must_use]
    pub fn as_of(&self) -> &str {
        &self.as_of
    }

    /// Where this table came from.
    #[must_use]
    pub fn source(&self) -> PriceTableSource {
        self.source
    }

    /// The same table tagged with another origin.
    #[must_use]
    pub fn with_source(mut self, source: PriceTableSource) -> Self {
        self.source = source;
        self
    }

    /// The model entries, sorted by id.
    #[must_use]
    pub fn models(&self) -> &[ModelPrice] {
        &self.models
    }

    /// Provenance for [`crate::model::Costs::price_table`].
    #[must_use]
    pub fn info(&self) -> PriceTableInfo {
        PriceTableInfo {
            as_of: self.as_of.clone(),
            source: self.source,
        }
    }

    /// The entry with the longest id that `model` equals or starts with.
    #[must_use]
    pub fn lookup(&self, model: &str) -> Option<&ModelPrice> {
        self.models
            .iter()
            .filter(|m| model.starts_with(m.id.as_str()))
            .max_by_key(|m| m.id.len())
    }

    /// Serializes to the TOML schema above, deterministically (models sorted by id), so the CLI can
    /// write the cache file. `parse(to_toml())` reproduces the table.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "as_of = \"{}\"\n", self.as_of);
        let _ = writeln!(out, "[cache_write]");
        let _ = writeln!(out, "five_minute = {:?}", self.cache_write_5m);
        let _ = writeln!(out, "one_hour = {:?}\n", self.cache_write_1h);
        let _ = writeln!(out, "[cache_read]");
        let _ = writeln!(out, "default_factor = {:?}", self.cache_read_factor);
        for m in &self.models {
            let _ = writeln!(out, "\n[models.\"{}\"]", escape(&m.id));
            let _ = writeln!(out, "input = {:?}", m.input);
            let _ = writeln!(out, "output = {:?}", m.output);
            for (key, v) in [
                ("cache_read", m.cache_read),
                ("cache_write_5m", m.cache_write_5m),
                ("cache_write_1h", m.cache_write_1h),
            ] {
                if let Some(v) = v {
                    let _ = writeln!(out, "{key} = {v:?}");
                }
            }
        }
        out
    }
}

const DEFAULT_W5: f64 = 1.25;
const DEFAULT_W1H: f64 = 2.0;
const DEFAULT_READ_FACTOR: f64 = 0.1;

fn check(what: &str, values: &[f64]) -> Result<(), PriceTableError> {
    if values.iter().all(|v| v.is_finite() && *v >= 0.0) {
        Ok(())
    } else {
        Err(PriceTableError::InvalidRate(what.to_owned()))
    }
}

/// Escapes a TOML basic-string body.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// Whether `s` is a real calendar date written `YYYY-MM-DD`.
pub(crate) fn valid_date(s: &str) -> bool {
    // ASCII first, so the byte-range slices below always fall on char boundaries.
    if !s.is_ascii() || s.len() != 10 || &s[4..5] != "-" || &s[7..8] != "-" {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = &s[range];
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let (Some(year), Some(month), Some(day)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return false;
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}
