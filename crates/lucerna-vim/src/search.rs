//! Search helpers: the word under the cursor, regex escaping and the
//! binary search for the Nth match. The `Vim` glue lives in `state.rs`.

use std::ops::Range;

use crate::classify::{CharClass, class};

/// Escapes the regex metacharacters (defensive: Word-class runs are alphanumeric or `_`) of `word` with a backslash.
pub(crate) fn escape_regex(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The byte range of the Word-class run under byte `col` of `line`; when
/// `col` is on a space or punctuation, the next such run on the line.
pub(crate) fn word_at(line: &str, col: u32) -> Option<Range<usize>> {
    let is_word = |c: char| class(c, false) == CharClass::Word;
    let col = (col as usize).min(line.len());
    let start = if line[col..].chars().next().is_some_and(is_word) {
        let before = line[..col].chars().rev().take_while(|&c| is_word(c));
        col - before.map(char::len_utf8).sum::<usize>()
    } else {
        col + line[col..].find(is_word)?
    };
    let len: usize = line[start..]
        .chars()
        .take_while(|&c| is_word(c))
        .map(char::len_utf8)
        .sum();
    Some(start..start + len)
}

/// The start of the `n`th match after (or before, when `backward`) the byte
/// `from`. Past either end it wraps when `wrap`, otherwise there is none.
pub(crate) fn nth_match(
    matches: &[Range<usize>],
    from: usize,
    backward: bool,
    n: u32,
    wrap: bool,
) -> Option<usize> {
    let len = matches.len();
    if len == 0 {
        return None;
    }
    let n = n.max(1) as usize;
    let index = if backward {
        // Matches that start before `from`; the nth one back.
        let before = matches.partition_point(|m| m.start < from);
        match before.checked_sub(n) {
            Some(i) => i,
            None if wrap => (before + len - n % len) % len,
            None => return None,
        }
    } else {
        let after = matches.partition_point(|m| m.start <= from);
        let i = after + n - 1;
        if i < len {
            i
        } else if wrap {
            i % len
        } else {
            return None;
        }
    };
    Some(matches[index].start)
}

#[cfg(test)]
mod tests {
    use lucerna_text::Point;

    use super::{escape_regex, nth_match, word_at};
    use crate::test_util::{TestView, assert_keys, assert_keys_with, feed_keys_with, run_keys};
    use crate::{Action, Key, Mode, Outcome, SearchQuery, Settings, ViewInfo, Vim, VimGlobals};

    fn query(pattern: &str) -> SearchQuery {
        SearchQuery {
            pattern: pattern.into(),
            is_regex: false,
            case_sensitive: true,
            whole_word: false,
        }
    }

    /// Feeds `prefix_keys` (ending in `/` or `?`), then submits `pattern`
    /// from the cursor like the workbench would.
    fn submit(
        view: &TestView,
        vim: &mut Vim,
        globals: &mut VimGlobals,
        prefix_keys: &str,
        pattern: &str,
    ) -> Outcome {
        let mut view = clone_view(view);
        let out = feed_keys_with(vim, &mut view, globals, prefix_keys);
        let Some(Outcome::Handled(actions)) = out.last() else {
            panic!("not handled: {out:?}")
        };
        let [Action::OpenSearch { backward }] = actions.as_slice() else {
            panic!("no OpenSearch: {actions:?}")
        };
        vim.search_submitted(
            query(pattern),
            *backward,
            vim.cursor(),
            &view,
            globals,
            &Settings::default(),
        )
    }

    fn clone_view(v: &TestView) -> TestView {
        TestView::new(&v.marked(v.cursor))
    }

    fn at(view: &TestView, vim: &Vim) -> String {
        view.marked(vim.cursor())
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can run a * search"
    #[test]
    fn star_jumps_to_next_whole_word() {
        assert_keys(
            "|blah blahblah duh blah blah",
            "*",
            "blah blahblah duh |blah blah",
        );
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "* ignores smartcase"
    #[test]
    fn star_ignores_smartcase() {
        assert_keys("|test TEST test", "*", "test |TEST test");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "* does not go over line boundaries"
    #[test]
    fn star_stays_within_the_line() {
        assert_keys(
            "one  |   \none two one two",
            "*",
            "one  |   \none two one two",
        );
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can handle # on whitespace"
    #[test]
    fn hash_on_whitespace_uses_next_word() {
        assert_keys("abc abcdef| abc", "#", "|abc abcdef abc");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can handle # already on the word"
    #[test]
    fn hash_inside_word_starts_from_word_start() {
        assert_keys("one o|ne", "#", "|one one");
    }

    #[test]
    fn star_emits_whole_word_highlight_and_jump() {
        let (_, _, actions) = run_keys(Settings::default(), |_| {}, "|foo.bar foo", "*");
        assert!(actions.contains(&Action::CursorMoved { jump: true }));
        assert!(actions.contains(&Action::Highlight(SearchQuery {
            pattern: "foo".into(),
            is_regex: true,
            case_sensitive: false,
            whole_word: true,
        })));
    }

    #[test]
    fn hash_then_n_keeps_going_backward() {
        // `#` from the last `a` goes to the middle one; `n` continues back.
        assert_keys("a a |a", "#n", "|a a a");
    }

    #[test]
    fn star_then_n_and_capital_n() {
        assert_keys("|a a a", "*n", "a a |a");
        assert_keys("|a a a", "*nN", "a |a a");
    }

    #[test]
    fn n_count_picks_nth_and_wraps() {
        assert_keys("|a a a a", "*4n", "a |a a a");
    }

    #[test]
    fn n_without_wrapscan_stops_at_last_match() {
        let settings = Settings {
            wrapscan: false,
            ..Settings::default()
        };
        assert_keys_with(settings, |_| {}, "|a a", "*n", "a |a");
    }

    #[test]
    fn star_without_wrapscan_and_no_later_match_stays() {
        let settings = Settings {
            wrapscan: false,
            ..Settings::default()
        };
        assert_keys_with(settings, |_| {}, "a |a", "*", "a |a");
    }

    #[test]
    fn n_without_search_does_nothing() {
        assert_keys("|a a", "n", "|a a");
    }

    #[test]
    fn n_is_a_jump_and_quote_quote_returns() {
        assert_keys("|a b a", "*n``", "a b |a");
        assert_keys("|a b a", "*''", "|a b a");
    }

    #[test]
    fn n_moves_visual_head() {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "|a b a", "v*");
        assert_eq!(vim.mode(), Mode::Visual);
        assert_eq!(view.marked(vim.cursor()), "a b |a");
    }

    #[test]
    fn n_with_pending_matches_retries_and_keeps_state() {
        let mut view = TestView::new("|a a a a a");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        let s = Settings::default();
        for c in ['*', '3'] {
            vim.feed(Key::Char(c), &view, &mut globals, &s);
        }
        let cursor = vim.cursor();
        assert_eq!(vim.pending(), "3");
        view.searching = true;
        let out = vim.feed(Key::Char('n'), &view, &mut globals, &s);
        assert!(matches!(out, Outcome::Retry(_)), "{out:?}");
        assert_eq!(vim.cursor(), cursor);
        assert_eq!(vim.pending(), "3");
        // Feeding the same key again once the matches are ready keeps the count.
        view.searching = false;
        vim.feed(Key::Char('n'), &view, &mut globals, &s);
        assert_eq!(view.marked(vim.cursor()), "a a a a |a");
    }

    #[test]
    fn submitted_with_pending_matches_retries_and_keeps_count() {
        let mut view = TestView::new("|a a a a a");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        for c in ['3', '/'] {
            vim.feed(Key::Char(c), &view, &mut g, &s);
        }
        view.searching = true;
        let out = vim.search_submitted(query("a"), false, vim.cursor(), &view, &mut g, &s);
        assert!(matches!(out, Outcome::Retry(_)), "{out:?}");
        assert_eq!(view.marked(vim.cursor()), "|a a a a a");
        assert!(g.last_search.is_none());
        view.searching = false;
        vim.search_submitted(query("a"), false, vim.cursor(), &view, &mut g, &s);
        assert_eq!(view.marked(vim.cursor()), "a a a |a a");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can run a search with count exceeding max number of matches and wrapscan"
    #[test]
    fn submitted_count_exceeding_matches_wraps() {
        let view = TestView::new("|one two two two");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        submit(&view, &mut vim, &mut g, "5/", "tw");
        assert_eq!(at(&view, &vim), "one two |two two");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can run a reverse and find next search"
    #[test]
    fn submitted_backward_then_n() {
        let view = TestView::new("one two two thre|e");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        submit(&view, &mut vim, &mut g, "?", "two");
        let mut v = clone_view(&view);
        feed_keys_with(&mut vim, &mut v, &mut g, "n");
        assert_eq!(at(&view, &vim), "one |two two three");
    }

    // Adapted from VSCodeVim test/search/search.test.ts: "?, match at EOL, walk over matches"
    #[test]
    fn submitted_backward_walks_over_matches() {
        let view = TestView::new("x end\nx\nx\n|start");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        submit(&view, &mut vim, &mut g, "?", "x");
        let mut v = clone_view(&view);
        feed_keys_with(&mut vim, &mut v, &mut g, "nn");
        assert_eq!(at(&view, &vim), "|x end\nx\nx\nstart");
    }

    #[test]
    fn submitted_search_is_a_jump_and_empty_pattern_is_ignored() {
        let view = TestView::new("|a b a");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let out = submit(&view, &mut vim, &mut g, "/", "a");
        assert_eq!(
            out,
            Outcome::Handled(vec![
                Action::CursorMoved { jump: true },
                Action::Highlight(query("a")),
            ])
        );
        let mut v = clone_view(&view);
        feed_keys_with(&mut vim, &mut v, &mut g, "''");
        assert_eq!(vim.cursor(), Point { row: 0, column: 0 });
        let out = submit(&view, &mut vim, &mut g, "/", "");
        assert_eq!(out, Outcome::Handled(vec![]));
        assert!(
            g.last_search
                .as_ref()
                .is_some_and(|(q, _)| q.pattern == "a")
        );
    }

    #[test]
    fn submitted_with_no_match_stays() {
        let view = TestView::new("|a b");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        submit(&view, &mut vim, &mut g, "/", "zzz");
        assert_eq!(at(&view, &vim), "|a b");
    }

    #[test]
    fn escape_and_word_helpers() {
        assert_eq!(escape_regex("a.b$c"), "a\\.b\\$c");
        assert_eq!(word_at("foo.bar", 1), Some(0..3));
        assert_eq!(word_at("foo.bar", 3), Some(4..7));
        assert_eq!(word_at("foo  ", 3), None);
        assert_eq!(nth_match(&[0..1, 4..5], 4, false, 1, true), Some(0));
        assert_eq!(nth_match(&[0..1, 4..5], 0, true, 1, false), None);
    }

    #[test]
    fn view_info_matches_is_literal() {
        let view = TestView::new("|ab ab");
        assert_eq!(view.matches(&query("ab")).unwrap().len(), 2);
    }
}
