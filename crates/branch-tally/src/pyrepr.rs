//! Python's `repr()` of a string, so messages quote a branch exactly as the reference's `{b!r}`.

use std::fmt::Write;

/// `repr(s)`: single quotes unless `s` holds a single quote and no double quote; backslash,
/// the chosen quote, `\n`, `\r` and `\t` escaped; other control characters as `\xNN`, `\uNNNN`
/// or `\UNNNNNNNN`; printable non-ASCII kept.
pub(crate) fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => {
                let n = u32::from(c);
                let _ = if n <= 0xff {
                    write!(out, "\\x{n:02x}")
                } else if n <= 0xffff {
                    write!(out, "\\u{n:04x}")
                } else {
                    write!(out, "\\U{n:08x}")
                };
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::repr;

    #[test]
    fn matches_python() {
        assert_eq!(repr("feat/x"), "'feat/x'");
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a\"b'c"), "'a\"b\\'c'");
        assert_eq!(repr("a\\b"), "'a\\\\b'");
        assert_eq!(repr("a\nb\x01\u{7f}"), "'a\\nb\\x01\\x7f'");
        assert_eq!(repr("caf\u{e9}"), "'caf\u{e9}'");
    }
}
