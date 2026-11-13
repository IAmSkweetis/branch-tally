//! [`Timestamp`]: a transcript or ledger timestamp that has been checked once, where it is read.
//!
//! Transcripts and ledger files are untrusted text, and a timestamp is rendered into PR bodies
//! and terminals. Holding it as a `String` let `|`, `[a](//x.co)`, `@user`, `<img src=x>` or a
//! control character reach the output unchecked. A `Timestamp` can only be built from strict
//! RFC 3339 shape, so what it holds is digits, `-`, `:`, `T`, `.`, `+` and `Z` only.
//!
//! The text is kept **exactly as read** (a ledger line round-trips byte for byte), and ordering
//! is by that text, as the reference script compared raw strings. Use [`Timestamp::instant`]
//! for ordering across offsets.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The text was not a strict RFC 3339 timestamp. It deliberately carries none of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTimestamp;

impl fmt::Display for InvalidTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an RFC 3339 timestamp")
    }
}

impl std::error::Error for InvalidTimestamp {}

/// A validated RFC 3339 timestamp: `YYYY-MM-DDTHH:MM:SS`, optional fractional seconds, then `Z`
/// or `+HH:MM` / `-HH:MM`. Fields are range-checked, including the day against the month.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp {
    // `text` is first so the derived `Ord` is the raw-string ordering the goldens rely on.
    text: String,
    instant: i128,
}

impl Timestamp {
    /// The timestamp as it was read.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Nanoseconds since the Unix epoch, offset applied. A leap second (`:60`) is the instant
    /// one second after `:59`.
    #[must_use]
    pub fn instant(&self) -> i128 {
        self.instant
    }
}

impl FromStr for Timestamp {
    type Err = InvalidTimestamp;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let instant = parse_instant(text.as_bytes()).ok_or(InvalidTimestamp)?;
        Ok(Self {
            text: text.to_owned(),
            instant,
        })
    }
}

/// Strict RFC 3339 over bytes, so a non-ASCII or control byte is simply "not a digit".
fn parse_instant(b: &[u8]) -> Option<i128> {
    let num = |from: usize, len: usize| -> Option<i64> {
        let digits = b.get(from..from + len)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| digits.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    };
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators.iter().any(|&(at, c)| b.get(at) != Some(&c)) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut at = 19;
    let mut nanos: i64 = 0;
    if b.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while b.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return None;
        }
        let mut scale = 100_000_000;
        for d in &b[start..at.min(start + 9)] {
            nanos += i64::from(d - b'0') * scale;
            scale /= 10;
        }
    }
    let offset_secs = match b.get(at)? {
        b'Z' if at + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if b.len() == at + 6 && b.get(at + 3) == Some(&b':') => {
            let (oh, om) = (num(at + 1, 2)?, num(at + 4, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = oh * 3600 + om * 60;
            if *sign == b'-' { -secs } else { secs }
        }
        _ => return None,
    };
    // Days from civil (proleptic Gregorian), Howard Hinnant's algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    Some(i128::from(secs) * 1_000_000_000 + i128::from(nanos))
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn valid_forms_parse_and_keep_their_text() {
        for text in [
            "2026-11-01T10:00:00Z",
            "2026-11-01T10:00:00.000Z",
            "2026-11-01T10:00:00.123456789012Z",
            "2026-11-01T10:00:00+01:00",
            "2026-11-01T10:00:00.5-05:30",
            "2024-02-29T23:59:59Z",
            "2026-12-31T23:59:60Z",
        ] {
            assert_eq!(ok(text).as_str(), text);
        }
    }

    #[test]
    fn instant_applies_the_offset() {
        assert_eq!(
            ok("2026-11-01T11:00:00+01:00").instant(),
            ok("2026-11-01T10:00:00Z").instant()
        );
        assert_eq!(ok("1970-01-01T00:00:00Z").instant(), 0);
        assert_eq!(ok("1970-01-01T00:00:01.5Z").instant(), 1_500_000_000);
    }

    #[test]
    fn hostile_text_is_refused() {
        for text in [
            "2026-11-01T10:00:00Z|",
            "2026-11-01T10:00:00Z [a](//x.co)",
            "[a](//x.co)",
            "@user",
            "2026-11-01T10:00:00Z@user",
            "<img src=x>",
            "2026-11-01T10:00:00Z<img src=x>",
            "2026-11-01T10:00:00Z\n",
            "2026-11-01T10:00:00\u{1b}[31mZ",
            "2026-11-01T10:00:00Z\u{202e}",
            "2026-11-01T10:00:00\u{0}Z",
        ] {
            assert!(
                text.parse::<Timestamp>().is_err(),
                "{text:?} must be refused"
            );
        }
    }

    #[test]
    fn malformed_shapes_are_refused() {
        for text in [
            "",
            "t",
            "2026-11-01",
            "2026-11-01T10:00",
            "2026-11-01T10:00:00",
            "2026-11-01 10:00:00Z",
            "2026-11-01t10:00:00Z",
            "2026-11-01T10:00:00z",
            "2026-11-01T10:00:00.Z",
            "2026-11-01T10:00:00+0100",
            "2026-11-01T10:00:00+01",
            "2026-11-01T10:00:00 +01:00",
            "26-11-01T10:00:00Z",
            "2026-1-01T10:00:00Z",
            "２０２６-11-01T10:00:00Z",
        ] {
            assert!(
                text.parse::<Timestamp>().is_err(),
                "{text:?} must be refused"
            );
        }
    }

    #[test]
    fn out_of_range_fields_are_refused() {
        for text in [
            "2026-00-01T10:00:00Z",
            "2026-13-01T10:00:00Z",
            "2026-11-00T10:00:00Z",
            "2026-11-31T10:00:00Z",
            "2026-02-29T10:00:00Z",
            "1900-02-29T10:00:00Z",
            "2026-11-01T24:00:00Z",
            "2026-11-01T10:60:00Z",
            "2026-11-01T10:00:61Z",
            "2026-11-01T10:00:00+24:00",
            "2026-11-01T10:00:00+01:60",
        ] {
            assert!(
                text.parse::<Timestamp>().is_err(),
                "{text:?} must be refused"
            );
        }
        assert!("2000-02-29T00:00:00Z".parse::<Timestamp>().is_ok());
    }

    #[test]
    fn the_error_carries_none_of_the_text() {
        let message = "@user".parse::<Timestamp>().unwrap_err().to_string();
        assert!(!message.contains("user"));
    }

    #[test]
    fn ordering_is_by_raw_text() {
        let a = ok("2026-11-01T10:00:00Z");
        let b = ok("2026-11-01T09:00:00-05:00");
        // 10:00Z is earlier as an instant, but the raw strings order the other way round.
        assert!(a.instant() < b.instant());
        assert_eq!(a.cmp(&b), a.as_str().cmp(b.as_str()));
        assert!(a > b);
    }

    #[test]
    fn serde_round_trips_the_text_and_refuses_hostile_input() {
        let t = ok("2026-11-01T10:00:00.250+01:00");
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#""2026-11-01T10:00:00.250+01:00""#);
        assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), t);
        assert!(serde_json::from_str::<Timestamp>(r#""@user""#).is_err());
        assert!(serde_json::from_str::<Timestamp>("5").is_err());
    }
}
