//! Redaction stage: turns a [`Summary`] into one that hides identifying detail.
//!
//! Runs after the summary stage and before pricing and render. Skeleton only for now; the logic
//! lands with the `redact` slice of #9.

use crate::model::Summary;

/// What to redact. `names: false` keeps agent and skill names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactOptions {
    /// Replace agent and skill names with generic labels.
    pub names: bool,
}

impl Default for RedactOptions {
    fn default() -> Self {
        Self { names: true }
    }
}

/// Returns `summary` with identifying detail replaced. Totals, buckets and the row order that
/// costs index by are unchanged.
#[must_use]
#[expect(
    clippy::todo,
    reason = "skeleton for the #9 redact slice; removed when it is implemented"
)]
pub fn redact(summary: Summary, opts: RedactOptions) -> Summary {
    let _ = (summary, opts);
    todo!("#9: redact slice")
}
