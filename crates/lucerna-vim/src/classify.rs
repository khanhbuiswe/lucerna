//! Character classes and per-line column helpers shared by the motions.

use lucerna_text::BufferSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CharClass {
    Word,
    Punct,
    Space,
}

/// The class of `c`. With `big_word` (`W`, `B`, `E`), punctuation counts
/// as part of a word.
// Word motions arrive in a later task; until then only tests call this.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn class(c: char, big_word: bool) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if big_word || c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// Horizontal reads stop after this many chars of a line.
pub(crate) const MAX_LINE_CHARS: usize = 10_000;

/// Row `row` without its line break, cut after [`MAX_LINE_CHARS`] chars.
pub(crate) fn line(s: &BufferSnapshot, row: u32) -> String {
    let mut text = s.line(row).into_owned();
    if let Some((cut, _)) = text.char_indices().nth(MAX_LINE_CHARS) {
        text.truncate(cut);
    }
    text
}

/// The display column of byte `byte_col`, with tabs expanded to `tab`.
pub(crate) fn display_col(line: &str, byte_col: u32, tab: u32) -> u32 {
    let tab = tab.max(1);
    let mut col = 0;
    for (i, c) in line.char_indices() {
        if i >= byte_col as usize {
            break;
        }
        col = advance(col, c, tab);
    }
    col
}

/// The byte column of the char covering display column `display`, clamped
/// to the last char of the line (0 for an empty line).
pub(crate) fn byte_col_for_display(line: &str, display: u32, tab: u32) -> u32 {
    let tab = tab.max(1);
    let mut col = 0;
    let mut last = 0;
    for (i, c) in line.char_indices() {
        last = i as u32;
        let next = advance(col, c, tab);
        if display < next {
            return last;
        }
        col = next;
    }
    last
}

/// The byte column of the last char, or 0 for an empty line.
pub(crate) fn last_char_col(line: &str) -> u32 {
    line.char_indices().next_back().map_or(0, |(i, _)| i as u32)
}

fn advance(col: u32, c: char, tab: u32) -> u32 {
    if c == '\t' {
        col + tab - col % tab
    } else {
        col + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Buffer;

    #[test]
    fn classes() {
        assert_eq!(class('a', false), CharClass::Word);
        assert_eq!(class('ế', false), CharClass::Word);
        assert_eq!(class('_', false), CharClass::Word);
        assert_eq!(class('.', false), CharClass::Punct);
        assert_eq!(class('😀', false), CharClass::Punct);
        assert_eq!(class('.', true), CharClass::Word);
        assert_eq!(class('\t', true), CharClass::Space);
    }

    #[test]
    fn line_strips_break_and_caps_length() {
        let s = Buffer::from_bytes(b"ab\r\ncd\n", false).unwrap().snapshot();
        assert_eq!(line(&s, 0), "ab");
        let long = "é".repeat(20_000);
        let s = Buffer::from_bytes(long.as_bytes(), false)
            .unwrap()
            .snapshot();
        assert_eq!(line(&s, 0).chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn display_columns_expand_tabs() {
        assert_eq!(display_col("\tab", 1, 4), 4);
        assert_eq!(display_col("a\tb", 2, 4), 4);
        assert_eq!(display_col("aé b", 3, 4), 2);
        assert_eq!(byte_col_for_display("\tab", 2, 4), 0);
        assert_eq!(byte_col_for_display("\tab", 5, 4), 2);
        assert_eq!(byte_col_for_display("ab", 9, 4), 1);
        assert_eq!(byte_col_for_display("", 3, 4), 0);
    }

    #[test]
    fn last_char() {
        assert_eq!(last_char_col(""), 0);
        assert_eq!(last_char_col("aé"), 1);
        assert_eq!(last_char_col("a😀"), 1);
    }
}
