//! Timestamp instants for ordering. A private copy of the attribution timeline parser
//! (`attribution::parse_ts` is not exported and `project` may not edit that module): keep the
//! two in step, or lift one shared helper into `model` later.

/// Nanoseconds since the Unix epoch for an RFC 3339 timestamp with an explicit offset
/// (`2026-11-01T10:00:00.000Z`, `...+01:00`). `None` for anything else: a timestamp whose
/// instant cannot be known is never guessed.
pub(super) fn parse_ts(text: &str) -> Option<i128> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let digits = b.get(from..from + len)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| digits.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    };
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    if !matches!(b.get(10), Some(b'T' | b't' | b' ')) || b.get(13) != Some(&b':') {
        return None;
    }
    if b.get(16) != Some(&b':') {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
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
        b'Z' | b'z' if at + 1 == b.len() => 0,
        sign @ (b'+' | b'-') => {
            let (oh, om) = match b.len() - at - 1 {
                2 => (num(at + 1, 2)?, 0),
                4 => (num(at + 1, 2)?, num(at + 3, 2)?),
                5 if b.get(at + 3) == Some(&b':') => (num(at + 1, 2)?, num(at + 4, 2)?),
                _ => return None,
            };
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
