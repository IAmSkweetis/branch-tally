//! The one timestamp-to-instant entry point for text that has not been typed yet (a capture
//! event's or a dispatch's `ts`). It is [`Timestamp`]'s strict parser, so there is a single
//! definition of a valid timestamp. Crate-internal: nothing outside core needs it.

use super::Timestamp;

/// Nanoseconds since the Unix epoch for a strict RFC 3339 timestamp, or `None` for anything
/// else: a timestamp whose instant cannot be known is never guessed.
pub(crate) fn parse_ts(text: &str) -> Option<i128> {
    text.parse::<Timestamp>().ok().map(|t| t.instant())
}
