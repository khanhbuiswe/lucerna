//! `.` (spec §6.1): the record of the last change and the steps that
//! replay it, and the typing recorded during Insert (spec §3.3).

use lucerna_text::{BufferSnapshot, Point};

use crate::classify::line;
use crate::visual::VimSelection;
use crate::{Key, SearchQuery};

/// One typing event of an Insert session, as the editor carried it out.
/// A replay sends it back through the same typing path, so auto-close and
/// type-over run again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// Typed chars, or the final text of an IME commit.
    Text(String),
    /// Enter.
    Newline,
    /// Backspace.
    DeleteLeft,
    /// Tab.
    Tab,
}

/// One step of an [`crate::Outcome::Replay`], run in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayStep {
    /// Feed this key to [`crate::Vim::feed`].
    Key(Key),
    /// Type this through the editor's typing path, without calling
    /// [`crate::Vim::record_typed`].
    Type(Typed),
    /// The search of an operator's `/` or `?` motion (`d/foo<CR>`): call
    /// [`crate::Vim::search_submitted`] with this query, `backward`, and
    /// vim's cursor as the origin, finding the matches at once and opening
    /// no find widget. The flags are those of the original search.
    Search {
        pattern: String,
        backward: bool,
        is_regex: bool,
        case_sensitive: bool,
        whole_word: bool,
    },
}

/// The selection a Visual change acted on, rebuilt from the cursor when
/// the change is repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// `v` within one row: this many chars.
    Chars(u32),
    /// `v` over `rows` rows, ending on the char `end_col` chars into the
    /// last one, or on its line break (`v$`) when `None`. A one-row `v$`
    /// is `rows: 1`.
    CharRows { rows: u32, end_col: Option<u32> },
    /// `V` over this many rows.
    Lines(u32),
}

impl Shape {
    /// The shape of `sel`; `None` for a block.
    pub(crate) fn of(sel: &VimSelection, s: &BufferSnapshot) -> Option<Shape> {
        let chars_before = |p: Point| {
            let text = line(s, p.row);
            let col = (p.column as usize).min(text.len());
            text.get(..col).map_or(0, |t| t.chars().count()) as u32
        };
        // The selection ends on the line break.
        let eol = |p: Point| p.column as usize >= crate::visual::line_len(s, p.row) as usize;
        match *sel {
            VimSelection::Char {
                start,
                end_inclusive,
            } if start.row == end_inclusive.row && !eol(end_inclusive) => Some(Shape::Chars(
                chars_before(end_inclusive) - chars_before(start) + 1,
            )),
            VimSelection::Char {
                start,
                end_inclusive,
            } => Some(Shape::CharRows {
                rows: end_inclusive.row - start.row + 1,
                end_col: (!eol(end_inclusive)).then(|| chars_before(end_inclusive)),
            }),
            VimSelection::Line { start_row, end_row } => {
                Some(Shape::Lines(end_row - start_row + 1))
            }
            VimSelection::Block { .. } => None,
        }
    }

    /// The keys that select this shape from the cursor in Normal mode.
    fn keys(self) -> Vec<Key> {
        let mut keys = Vec::new();
        match self {
            Shape::Chars(n) => {
                keys.push(Key::Char('v'));
                push_counted(&mut keys, n - 1, 'l');
            }
            Shape::CharRows { rows, end_col } => {
                keys.push(Key::Char('v'));
                push_counted(&mut keys, rows - 1, 'j');
                match end_col {
                    Some(col) => {
                        keys.push(Key::Char('0'));
                        push_counted(&mut keys, col, 'l');
                    }
                    None => keys.push(Key::Char('$')),
                }
            }
            Shape::Lines(rows) => {
                keys.push(Key::Char('V'));
                push_counted(&mut keys, rows - 1, 'j');
            }
        }
        keys
    }
}

/// `n` then `motion`; nothing when `n` is 0.
fn push_counted(keys: &mut Vec<Key>, n: u32, motion: char) {
    if n > 0 {
        push_count(keys, n);
        keys.push(Key::Char(motion));
    }
}

fn push_count(keys: &mut Vec<Key>, n: u32) {
    keys.extend(n.to_string().chars().map(Key::Char));
}

/// The last change, for `.` (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LastChange {
    /// The register chosen with `"x`.
    pub register: Option<char>,
    /// The command's count: the product of the counts before and after an
    /// operator.
    pub count: Option<u32>,
    /// The command's keys, without the register and the counts (and
    /// without the `/` or `?` of a search motion).
    pub keys: Vec<Key>,
    /// The query and direction of an operator's `/` or `?` motion, run
    /// after `keys` with no find widget. The whole query is kept, so the
    /// repeat uses the flags (regex, case, whole word) of the original.
    pub search: Option<(SearchQuery, bool)>,
    /// The selection of a Visual change.
    pub shape: Option<Shape>,
    /// The command ended in Insert: the typing until Escape.
    pub insert: bool,
    pub typed: Vec<Typed>,
}

impl LastChange {
    /// The text the Insert session typed, for the `.` register.
    pub(crate) fn inserted_text(&self) -> String {
        self.typed
            .iter()
            .filter_map(|t| match t {
                Typed::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The steps that repeat the change, with `count` in place of the
    /// recorded one when given. A numbered register `1`–`8` becomes the
    /// next one, so `"1p...` puts `"1`, `"2`, `"3`, `"4` in turn.
    pub(crate) fn replay(&self, count: Option<u32>) -> Vec<ReplayStep> {
        let mut keys = self.shape.map_or_else(Vec::new, Shape::keys);
        if let Some(r) = self.register {
            let r = match r {
                '1'..='8' => (r as u8 + 1) as char,
                _ => r,
            };
            keys.push(Key::Char('"'));
            keys.push(Key::Char(r));
        }
        if let Some(n) = count.or(self.count) {
            push_count(&mut keys, n);
        }
        keys.extend(&self.keys);
        let mut steps: Vec<ReplayStep> = keys.into_iter().map(ReplayStep::Key).collect();
        if let Some((query, backward)) = &self.search {
            steps.push(ReplayStep::Search {
                pattern: query.pattern.clone(),
                backward: *backward,
                is_regex: query.is_regex,
                case_sensitive: query.case_sensitive,
                whole_word: query.whole_word,
            });
        }
        if self.insert {
            steps.extend(self.typed.iter().cloned().map(ReplayStep::Type));
            steps.push(ReplayStep::Key(Key::Escape));
        }
        steps
    }
}

#[cfg(test)]
mod tests {
    use lucerna_editor::EditConfig;

    use super::*;
    use crate::test_util::{
        TestView, assert_edit, assert_edit_with, parse_keys, run_edit, run_edit_with,
    };
    use crate::{Action, Outcome, SearchQuery, Settings, Vim, VimGlobals};

    fn tabs(view: &mut TestView) {
        view.config = EditConfig {
            insert_spaces: false,
            ..view.config.clone()
        };
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/dot.test.ts: "Repeat
    // with count replaces the count of the change"
    #[test]
    fn repeat_count_replaces_the_recorded_count() {
        assert_edit("one |two three four five", "dw2.", "one |five");
        assert_edit("one |two three four five", "d2w.", "one| ");
        // Counts before and after the operator multiply.
        assert_edit("|a b c d e f g", "2d1w3.", "|f g");
        // `3.` is remembered: the next `.` deletes 3 words again.
        assert_edit("|a b c d e f g h", "dw3..", "|h");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle dot
    // with A"
    #[test]
    fn repeat_append() {
        assert_edit("|one\ntwo\nthree", "A!<Esc>j.j.", "one!\ntwo!\nthree|!");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/dot.test.ts:
    // "Repeat insert with count"; the start has a real second line, since
    // `j` never reaches the empty row after a final line break.
    #[test]
    fn repeat_insert_with_count_multiplies_the_insert() {
        assert_edit("|\n\n", "ivar<Esc>j4.", "var\nvarvarvarva|r\n");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/dot.test.ts: "Repeat
    // with numbered registers"
    #[test]
    fn repeat_steps_through_numbered_registers() {
        assert_edit("|1 2 3", "\"1daw..\"1p..", "1 2 |3");
        // A named register stays the same.
        assert_edit("|ab", "\"ax.\"ap", "|b");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can repeat
    // shift in visual line mode"
    #[test]
    fn repeat_visual_line_shift() {
        assert_edit_with(
            Settings::default(),
            tabs,
            "on|e\ntwo",
            "Vj>.",
            "\t\t|one\n\t\ttwo",
        );
    }

    #[test]
    fn repeat_visual_char_change_over_the_same_shape() {
        // Three chars on one row.
        assert_edit("|abcdefg", "vllcX<Esc>l.", "X|Xg");
        // Two rows, ending two chars into the last one.
        assert_edit("|ab\ncde\nfgh\nijk\nlm", "vjlldj.", "\n|\nlm");
        // Counted visual `>` repeats with its count.
        assert_edit_with(Settings::default(), tabs, "|a\nb", "v2>j.", "\t\ta\n\t\t|b");
    }

    #[test]
    fn repeat_visual_to_the_line_end() {
        assert_edit("|ab\ncd\nef", "v$d.", "|ef");
        assert_edit("|a\nbb\nc\nd\ne", "vj$d.", "|e");
        // From the middle of a row, `$` still reaches its line break.
        assert_edit("x|abc\nd\nef\ng", "v$dj.", "xd\ne|g");
    }

    #[test]
    fn failed_insert_command_in_a_replay_types_nothing() {
        // `.` repeats `ci(` where there is no pair: nothing is typed.
        assert_edit("(|x) y", "ci(foo<Esc>$.", "(foo) |y");
        // The unused typing does not leak into the next Insert session.
        assert_edit("(|x) y", "ci(foo<Esc>$.iz<Esc>0.", "|z(foo) zy");
    }

    #[test]
    fn counted_and_register_insert_changes_repeat_twice() {
        assert_edit("|a b c d e f", "2cwX<Esc>w.w.", "X X |X");
        assert_edit("|a b c", "\"acwY<Esc>w.w.", "Y Y |Y");
        assert_edit("|a b c", "\"a2cwY<Esc>w.", "Y |Y");
    }

    #[test]
    fn repeat_change_and_operators() {
        assert_edit("|one two three", "cwX<Esc>w.", "X |X three");
        assert_edit("|abc abc", "x.", "|c abc");
        assert_edit("|a\nb\nc\n", "dd.", "|c\n");
        assert_edit("|ab ab", "rxw.", "xb |xb");
        assert_edit("|a\nb", "ofoo<Esc>j.", "a\nfoo\nb\nfo|o");
        assert_edit("|abc", "yl.", "|abc");
    }

    #[test]
    fn repeat_skips_motions_yank_undo_and_no_op_changes() {
        // `l`, `yl` and `u` leave `x` as the change to repeat.
        assert_edit("|abcde", "xlylu.", "|bcde");
        // `i<Esc>` typed nothing: `.` still repeats `x`.
        assert_edit("|abcd", "xi<Esc>.", "|cd");
        // Nothing to repeat yet.
        assert_edit("|ab", ".", "|ab");
    }

    #[test]
    fn repeat_replays_the_recorded_steps() {
        let mut view = TestView::new("|ab");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        for k in parse_keys("\"a2x") {
            if let Outcome::Handled(a) = vim.feed(k, &view, &mut g, &s) {
                view.apply(&mut vim, &a);
            }
        }
        let out = vim.feed(Key::Char('.'), &view, &mut g, &s);
        let keys = parse_keys("\"a2x")
            .into_iter()
            .map(ReplayStep::Key)
            .collect();
        assert_eq!(out, Outcome::Replay(keys));
        let out = vim.feed(Key::Char('3'), &view, &mut g, &s);
        assert_eq!(out, Outcome::Handled(vec![]));
        let out = vim.feed(Key::Char('.'), &view, &mut g, &s);
        let keys = parse_keys("\"a3x")
            .into_iter()
            .map(ReplayStep::Key)
            .collect();
        assert_eq!(out, Outcome::Replay(keys));
    }

    #[test]
    fn search_operator_replays_its_search() {
        let mut view = TestView::new("|ab foo");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        for k in parse_keys("\"a2d/") {
            if let Outcome::Handled(a) = vim.feed(k, &view, &mut g, &s) {
                view.apply(&mut vim, &a);
            }
        }
        assert_eq!(vim.pending(), "\"a2d/");
        let query = SearchQuery {
            pattern: "o".into(),
            is_regex: false,
            case_sensitive: true,
            whole_word: false,
        };
        let Outcome::Handled(a) =
            vim.search_submitted(query, false, vim.cursor(), &view, &mut g, &s)
        else {
            panic!()
        };
        view.apply(&mut vim, &a);
        assert_eq!(view.marked(vim.cursor()), "|o");
        let out = vim.feed(Key::Char('.'), &view, &mut g, &s);
        let mut steps: Vec<ReplayStep> = parse_keys("\"a2d")
            .into_iter()
            .map(ReplayStep::Key)
            .collect();
        steps.push(ReplayStep::Search {
            pattern: "o".into(),
            backward: false,
            is_regex: false,
            case_sensitive: true,
            whole_word: false,
        });
        assert_eq!(out, Outcome::Replay(steps));
    }

    #[test]
    fn search_operator_repeats_with_the_original_flags() {
        let mut view = TestView::new("|a foo Foo\nb foo Foo");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        let query = SearchQuery {
            pattern: "Foo".into(),
            is_regex: true,
            case_sensitive: true,
            whole_word: true,
        };
        for k in parse_keys("d/") {
            vim.feed(k, &view, &mut g, &s);
        }
        let Outcome::Handled(a) =
            vim.search_submitted(query.clone(), false, vim.cursor(), &view, &mut g, &s)
        else {
            panic!()
        };
        view.apply(&mut vim, &a);
        assert_eq!(view.marked(vim.cursor()), "|Foo\nb foo Foo");
        for k in parse_keys("j0") {
            if let Outcome::Handled(a) = vim.feed(k, &view, &mut g, &s) {
                view.apply(&mut vim, &a);
            }
        }
        let Outcome::Replay(steps) = vim.feed(Key::Char('.'), &view, &mut g, &s) else {
            panic!()
        };
        assert_eq!(
            steps.last(),
            Some(&ReplayStep::Search {
                pattern: "Foo".into(),
                backward: false,
                is_regex: true,
                case_sensitive: true,
                whole_word: true,
            })
        );
        for step in steps {
            let out = match step {
                ReplayStep::Key(k) => vim.feed(k, &view, &mut g, &s),
                ReplayStep::Search { backward, .. } => {
                    vim.search_submitted(query.clone(), backward, vim.cursor(), &view, &mut g, &s)
                }
                ReplayStep::Type(_) => panic!(),
            };
            if let Outcome::Handled(a) = out {
                view.apply(&mut vim, &a);
            }
        }
        // Case-sensitive: the lowercase `foo` is skipped.
        assert_eq!(view.marked(vim.cursor()), "Foo\n|Foo");
    }

    #[test]
    fn operator_pending_reports_a_waiting_operator() {
        let view = TestView::new("|a foo");
        let mut vim = Vim::new(view.cursor);
        assert!(!vim.operator_pending());
        let mut g = VimGlobals::default();
        vim.feed(Key::Char('d'), &view, &mut g, &Settings::default());
        assert!(vim.operator_pending());
    }

    #[test]
    fn insert_replay_carries_the_typing_then_escape() {
        let mut view = TestView::new("|x");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        let out = vim.feed(Key::Char('i'), &view, &mut g, &s);
        let Outcome::Handled(a) = out else { panic!() };
        view.apply(&mut vim, &a);
        vim.record_typed(Typed::Text("a".into()));
        vim.record_typed(Typed::Newline);
        vim.record_typed(Typed::DeleteLeft);
        vim.record_typed(Typed::Tab);
        vim.feed(Key::Escape, &view, &mut g, &s);
        let out = vim.feed(Key::Char('.'), &view, &mut g, &s);
        assert_eq!(
            out,
            Outcome::Replay(vec![
                ReplayStep::Key(Key::Char('i')),
                ReplayStep::Type(Typed::Text("a".into())),
                ReplayStep::Type(Typed::Newline),
                ReplayStep::Type(Typed::DeleteLeft),
                ReplayStep::Type(Typed::Tab),
                ReplayStep::Key(Key::Escape),
            ])
        );
    }

    #[test]
    fn dot_register_holds_the_last_insert_text() {
        let mut view = TestView::new("|x");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        let path = view.path.clone();
        let read = |g: &VimGlobals| {
            g.read_register(Some('.'), s.use_system_clipboard, &path)
                .map(|c| c.text)
        };
        assert_eq!(read(&g), None);
        let Outcome::Handled(a) = vim.feed(Key::Char('a'), &view, &mut g, &s) else {
            panic!()
        };
        view.apply(&mut vim, &a);
        vim.record_typed(Typed::Text("fo".into()));
        vim.record_typed(Typed::Newline);
        vim.record_typed(Typed::Text("o".into()));
        vim.feed(Key::Escape, &view, &mut g, &s);
        assert_eq!(read(&g), Some("foo".into()));
        // `".p` puts it.
        assert_edit("|x", "ifoo<Esc>\".p", "foofo|ox");
    }

    #[test]
    fn break_insert_record_starts_a_new_record() {
        let mut view = TestView::new("|x\ny");
        let mut vim = Vim::new(view.cursor);
        let mut g = VimGlobals::default();
        let s = Settings::default();
        let Outcome::Handled(a) = vim.feed(Key::Char('A'), &view, &mut g, &s) else {
            panic!()
        };
        view.apply(&mut vim, &a);
        vim.record_typed(Typed::Text("a".into()));
        vim.break_insert_record();
        vim.record_typed(Typed::Text("b".into()));
        vim.feed(Key::Escape, &view, &mut g, &s);
        let out = vim.feed(Key::Char('.'), &view, &mut g, &s);
        assert_eq!(
            out,
            Outcome::Replay(vec![
                ReplayStep::Key(Key::Char('i')),
                ReplayStep::Type(Typed::Text("b".into())),
                ReplayStep::Key(Key::Escape),
            ])
        );
        // A break with nothing typed after it keeps the change before it.
        let mut vim = Vim::new(view.cursor);
        let Outcome::Handled(a) = vim.feed(Key::Char('A'), &view, &mut g, &s) else {
            panic!()
        };
        view.apply(&mut vim, &a);
        vim.record_typed(Typed::Text("c".into()));
        vim.break_insert_record();
        vim.feed(Key::Escape, &view, &mut g, &s);
        let out = vim.feed(Key::Char('.'), &view, &mut g, &s);
        assert_eq!(
            out,
            Outcome::Replay(vec![
                ReplayStep::Key(Key::Char('A')),
                ReplayStep::Type(Typed::Text("c".into())),
                ReplayStep::Key(Key::Escape),
            ])
        );
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/undo.test.ts: "Can handle u"
    #[test]
    fn undo_steps_back() {
        assert_edit("one |two three four five", "dwdwu", "one |three four five");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/undo.test.ts: "Can handle
    // <C-r>"
    #[test]
    fn undo_with_count_then_redo() {
        assert_edit(
            "one |two three four five",
            "dwdw2u<C-r>",
            "one |three four five",
        );
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/undo.test.ts: "Can handle undo
    // delete with visual mode"
    #[test]
    fn undo_visual_delete() {
        assert_edit(
            "one |two three four five",
            "vwwdu",
            "one |two three four five",
        );
    }

    #[test]
    fn undo_keys_emit_actions_and_are_normal_only() {
        let (_, actions) = run_edit("|a", "3u2<C-r>");
        assert!(actions.contains(&Action::Undo(3)));
        assert!(actions.contains(&Action::Redo(2)));
        // Visual `u` lowercases.
        assert_edit("|AB", "vlu", "|ab");
        // A read-only view gets no undo.
        let (_, actions) =
            run_edit_with(Settings::default(), |v| v.readonly = true, "|a", "u<C-r>");
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::Undo(_) | Action::Redo(_)))
        );
    }

    #[test]
    fn a_replay_is_one_undo_step() {
        assert_edit("|a b c d", "dw2.u", "|b c d");
        assert_edit("|x\ny", "Afoo<Esc>j.u", "xfoo\n|y");
    }

    #[test]
    fn change_inside_brackets_replays_through_autoclose() {
        assert_edit("(|x) (y)", "ci(foo(bar<Esc>f(.", "(foo(bar)) (foo(ba|r))");
    }

    #[test]
    fn counted_insert_types_the_text_count_times() {
        assert_edit("|", "3ifoo<Esc>", "foofoofo|o");
        assert_edit("|", "3ifoo<Esc>.", "foofoofofoofoofo|oo");
        assert_edit("|a", "2ofoo<Esc>", "a\nfoo\nfo|o");
        // The count does not leak into the next Insert session.
        assert_edit("|", "3ia<Esc>ib<Esc>", "aa|ba");
    }
}
