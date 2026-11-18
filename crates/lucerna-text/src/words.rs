//! Character classes and the whole-word rule shared by find and project
//! search.
//!
//! Adapted from microsoft/vscode src/vs/editor/common/core/wordCharacterClassifier.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/common/model/textModelSearch.ts (MIT)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Regular,
    Whitespace,
    Separator,
}

/// Space and tab are whitespace, chars of `separators` are separators,
/// everything else is regular.
pub fn classify(c: char, separators: &str) -> CharClass {
    if c == ' ' || c == '\t' {
        CharClass::Whitespace
    } else if separators.contains(c) {
        CharClass::Separator
    } else {
        CharClass::Regular
    }
}

/// Whether a match is bounded by word separators or whitespace on both
/// sides, as VS Code's `isValidMatch` decides it. `before` and `after` are
/// the chars just outside the match (`None` at the text edge); `first` and
/// `last` are the match's own edge chars. A side is a boundary at the text
/// edge, when the neighbouring char is not a word char, or when the match's
/// own edge char is not one. Line breaks are never word chars.
pub fn is_word_bounded(
    before: Option<char>,
    first: Option<char>,
    last: Option<char>,
    after: Option<char>,
    separators: &str,
) -> bool {
    let regular = |c: char| c != '\n' && c != '\r' && classify(c, separators) == CharClass::Regular;
    let left = before.is_none_or(|c| !regular(c)) || first.is_some_and(|c| !regular(c));
    let right = after.is_none_or(|c| !regular(c)) || last.is_some_and(|c| !regular(c));
    left && right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_by_separators_and_edges() {
        let seps = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
        assert!(is_word_bounded(None, Some('f'), Some('o'), Some(' '), seps));
        assert!(!is_word_bounded(
            Some('a'),
            Some('f'),
            Some('o'),
            None,
            seps
        ));
        assert!(is_word_bounded(
            Some('-'),
            Some('f'),
            Some('o'),
            Some('-'),
            seps
        ));
        assert!(!is_word_bounded(
            Some('-'),
            Some('f'),
            Some('o'),
            Some('-'),
            ""
        ));
        assert!(is_word_bounded(
            Some('\n'),
            Some('f'),
            Some('o'),
            Some('\r'),
            seps
        ));
        // The match's own edge char counts: "-foo" inside "a-foo".
        assert!(is_word_bounded(Some('a'), Some('-'), Some('o'), None, seps));
        assert!(!is_word_bounded(
            Some('x'),
            Some('đ'),
            Some('g'),
            Some(' '),
            " "
        ));
    }
}
