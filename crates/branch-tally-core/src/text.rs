//! Shared text utilities: small, pure helpers every output stage can use.

use std::borrow::Cow;

/// What a control character is replaced with: U+FFFD, visible and inert.
pub const REPLACEMENT: char = '\u{FFFD}';

/// Whether `c` can change how a terminal or a Markdown viewer lays out or reads text: C0 and C1
/// controls, DEL, and the Unicode bidi marks, overrides and isolates.
#[must_use]
pub fn is_unsafe_control(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{61c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// `text` with every [`is_unsafe_control`] character replaced by [`REPLACEMENT`], one for one.
/// Borrows when there is nothing to replace. Tabs and newlines are controls too: a caller that
/// wants line breaks must split before calling.
#[must_use]
pub fn replace_control_chars(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_unsafe_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| if is_unsafe_control(c) { REPLACEMENT } else { c })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed_unchanged() {
        let out = replace_control_chars("feat/x \u{e9} \u{4e2d} | `a`");
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "feat/x \u{e9} \u{4e2d} | `a`");
    }

    #[test]
    fn c0_c1_and_del_are_replaced() {
        for c in ('\u{0}'..='\u{1f}').chain('\u{7f}'..='\u{9f}') {
            let input = format!("a{c}b");
            let out = replace_control_chars(&input);
            assert_eq!(out, "a\u{FFFD}b", "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn bidi_controls_are_replaced() {
        for c in ['\u{200e}', '\u{200f}', '\u{061c}']
            .into_iter()
            .chain('\u{202a}'..='\u{202e}')
            .chain('\u{2066}'..='\u{2069}')
        {
            let input = format!("a{c}b");
            let out = replace_control_chars(&input);
            assert_eq!(out, "a\u{FFFD}b", "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn an_escape_sequence_loses_its_escape_only() {
        assert_eq!(replace_control_chars("\u{1b}[31mred"), "\u{FFFD}[31mred");
    }

    #[test]
    fn neighbouring_characters_are_kept() {
        assert!(!is_unsafe_control('\u{200d}'), "zero-width joiner");
        assert!(!is_unsafe_control('\u{a0}'));
        assert!(!is_unsafe_control('\u{a1}'));
        assert!(!is_unsafe_control('\u{2065}'));
        assert!(!is_unsafe_control('\u{202f}'));
        assert_eq!(replace_control_chars("\n\t"), "\u{FFFD}\u{FFFD}");
        assert_eq!(replace_control_chars(""), "");
    }
}
