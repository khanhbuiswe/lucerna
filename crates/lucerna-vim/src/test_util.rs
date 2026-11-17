//! String-based test helpers: `|` marks the cursor.
//!
//! [`run_edit`] (and the `assert_edit` helpers) drive vim the way the
//! workbench does. While vim is in Insert mode, the keys type instead of
//! going to vim, through the editor's typing path (`commands::insert`
//! and friends), and each is recorded with [`Vim::record_typed`]: a char
//! types itself (`Typed::Text` of that one char, so auto-close runs),
//! `<CR>` is `Typed::Newline`, `<BS>` `Typed::DeleteLeft` and `<Tab>`
//! `Typed::Tab`. `<Esc>` and ctrl keys still go to vim. An
//! [`Outcome::Replay`] runs its steps at once, in one undo step, without
//! recording the typing, and stops at a `Type` step that comes outside
//! Insert mode. A command that enters Insert and the typing up
//! to Escape are one undo step too.
//!
//! After a key that opens the search (`/`, `?`), the keys type the
//! pattern like the find widget: `<CR>` submits it with
//! [`Vim::search_submitted`] (a literal, case-insensitive query from vim's
//! cursor when the search opened), `<Esc>` dismisses it like the
//! workbench, `<BS>` deletes a char.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use lucerna_editor::{Ctx, EditConfig};
use lucerna_text::{Buffer, BufferSnapshot, Point};

use crate::classify::{CharClass, class};
use crate::{
    Action, InsertAt, Key, Outcome, ReplayStep, SearchQuery, Settings, Typed, ViewInfo, Vim,
    VimGlobals,
};

pub(crate) struct TestView {
    pub path: Arc<Path>,
    pub snapshot: BufferSnapshot,
    pub cursor: Point,
    pub top_row: u32,
    pub visible_rows: u32,
    /// Rows inside closed folds.
    pub hidden: Vec<Range<u32>>,
    pub tab_size: u32,
    /// Makes `matches` return `None`, like a search still running.
    pub searching: bool,
    /// Makes `edit_ctx` return `None`, like a read-only view.
    pub readonly: bool,
    pub config: EditConfig,
    /// Syntax nodes for `enclosing_nodes`, as byte ranges.
    pub nodes: Vec<Range<usize>>,
    /// How many times `enclosing_nodes` was called.
    pub node_queries: std::cell::Cell<usize>,
    /// Undo steps, oldest first, and redo steps, latest undone last.
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    /// An open undo group: edits until it closes are one step.
    group: Option<Group>,
}

/// Undo steps from `start` on merge into one that begins at `before`.
struct Group {
    depth: u32,
    start: usize,
    before: (BufferSnapshot, Point),
}

/// One feed's edits: the text and vim's cursor before and after.
struct UndoStep {
    before: (BufferSnapshot, Point),
    after: (BufferSnapshot, Point),
}

impl TestView {
    /// `text` holds exactly one `|`, the cursor.
    pub fn new(text: &str) -> TestView {
        let at = text.find('|').expect("no `|` cursor marker");
        assert_eq!(text.matches('|').count(), 1, "more than one `|`");
        let plain = format!("{}{}", &text[..at], &text[at + 1..]);
        let snapshot = Buffer::from_bytes(plain.as_bytes(), false)
            .unwrap()
            .snapshot();
        let cursor = snapshot.offset_to_point(at);
        TestView {
            path: Arc::from(Path::new("test.rs")),
            snapshot,
            cursor,
            top_row: 0,
            visible_rows: 30,
            hidden: Vec::new(),
            tab_size: 4,
            searching: false,
            readonly: false,
            config: lucerna_editor::test_util::test_config(),
            nodes: Vec::new(),
            node_queries: std::cell::Cell::new(0),
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
        }
    }

    /// Carries out the actions of one feed the way the workbench does:
    /// every `Edit` changes the text (all of them one undo step), `Undo`
    /// and `Redo` walk the history and put the cursor where the step
    /// started or ended, and `ScrollTopRow` moves the view. Vim's cursor is
    /// clamped to the new text afterwards.
    pub fn apply(&mut self, vim: &mut Vim, actions: &[Action]) {
        let before = (self.snapshot.clone(), vim.cursor());
        debug_assert!(
            actions
                .iter()
                .filter(|a| matches!(a, Action::Edit(_)))
                .count()
                <= 1,
            "a feed emits at most one Action::Edit: {actions:?}"
        );
        let mut edited = None;
        for action in actions {
            match action {
                Action::Edit(plan) => {
                    debug_assert!(
                        plan.edits.windows(2).all(|w| w[0].0.end <= w[1].0.start),
                        "edits must be ascending and non-overlapping: {:?}",
                        plan.edits
                    );
                    let mut text = self.snapshot.text_for_range(0..self.snapshot.len_bytes());
                    for (range, new) in plan.edits.iter().rev() {
                        text.replace_range(range.clone(), new);
                    }
                    self.snapshot = BufferSnapshot::from_text(&text);
                    edited = Some(plan.selections.primary().head);
                }
                // Undo puts vim's cursor back where it was before the
                // command (the view's selections before it, like
                // VSCodeVim's cursorsAtStart); redo at the plan's head.
                Action::Undo(n) => {
                    self.commit(&mut edited, &before);
                    for _ in 0..*n {
                        let Some(step) = self.undo.pop() else { break };
                        (self.snapshot, _) = step.before.clone();
                        vim.reset(step.before.1);
                        self.redo.push(step);
                    }
                }
                Action::Redo(n) => {
                    self.commit(&mut edited, &before);
                    for _ in 0..*n {
                        let Some(step) = self.redo.pop() else { break };
                        (self.snapshot, _) = step.after.clone();
                        vim.reset(step.after.1);
                        self.undo.push(step);
                    }
                }
                Action::ScrollTopRow(row) => self.top_row = *row,
                // Like the workbench, `o` and `O` open a line that keeps
                // the indent, and the caret goes there.
                Action::EnterInsert(at @ (InsertAt::LineBelow | InsertAt::LineAbove)) => {
                    let row = vim.cursor().row;
                    let line = self.snapshot.line(row).to_string();
                    let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
                    let (offset, text, caret) = if *at == InsertAt::LineBelow {
                        let end = self.snapshot.point_to_offset(Point {
                            row,
                            column: line.len() as u32,
                        });
                        (
                            end,
                            format!("\n{indent}"),
                            Point {
                                row: row + 1,
                                column: indent.len() as u32,
                            },
                        )
                    } else {
                        let start = self.snapshot.point_to_offset(Point { row, column: 0 });
                        (
                            start,
                            format!("{indent}\n"),
                            Point {
                                row,
                                column: indent.len() as u32,
                            },
                        )
                    };
                    let mut all = self.snapshot.text_for_range(0..self.snapshot.len_bytes());
                    all.insert_str(offset, &text);
                    self.snapshot = BufferSnapshot::from_text(&all);
                    edited = Some(self.snapshot.point_to_offset(caret));
                    vim.set_cursor(caret);
                }
                _ => {}
            }
        }
        self.commit(&mut edited, &before);
        vim.clamp(&self.snapshot);
    }

    /// Records the pending edits (whose plan left the cursor at `head`) as
    /// one undo step.
    fn commit(&mut self, edited: &mut Option<usize>, before: &(BufferSnapshot, Point)) {
        let Some(head) = edited.take() else { return };
        let after = (self.snapshot.clone(), self.snapshot.offset_to_point(head));
        self.redo.clear();
        if let Some(g) = &self.group {
            if self.undo.len() > g.start {
                self.undo.last_mut().expect("a grouped step").after = after;
                return;
            }
            let before = g.before.clone();
            self.undo.push(UndoStep { before, after });
            return;
        }
        self.undo.push(UndoStep {
            before: before.clone(),
            after,
        });
    }

    /// Opens an undo group (or nests in the open one) that starts with the
    /// text and `cursor` as they are now.
    pub fn begin_group(&mut self, cursor: Point) {
        match &mut self.group {
            Some(g) => g.depth += 1,
            None => {
                self.group = Some(Group {
                    depth: 1,
                    start: self.undo.len(),
                    before: (self.snapshot.clone(), cursor),
                })
            }
        }
    }

    pub fn end_group(&mut self) {
        if let Some(g) = &mut self.group {
            g.depth -= 1;
            if g.depth == 0 {
                self.group = None;
            }
        }
    }

    /// Types `typed` at vim's cursor the way the editor does, and moves
    /// vim's cursor to the caret after it. Auto-closed pairs are not
    /// tracked, so typing over a closer types a new one.
    pub fn type_typed(&mut self, vim: &mut Vim, typed: &Typed) {
        use lucerna_editor::{EditKind, SelectionSet, commands};
        let Some(ctx) = self.edit_ctx() else { return };
        let at = self.snapshot.point_to_offset(vim.cursor());
        let set = SelectionSet::cursor(at);
        let plan = match typed {
            Typed::Text(text) => commands::insert(ctx, &set, text, EditKind::Other, &[]),
            Typed::Newline => commands::newline(ctx, &set),
            Typed::DeleteLeft => commands::delete_left(ctx, &set, &[]),
            Typed::Tab => commands::tab(ctx, &set),
        };
        let head = plan.selections.primary().head;
        self.apply(vim, &[Action::Edit(plan)]);
        vim.set_cursor(self.snapshot.offset_to_point(head));
    }

    /// The text with `|` inserted at `cursor`.
    pub fn marked(&self, cursor: Point) -> String {
        let at = self.snapshot.point_to_offset(cursor);
        let len = self.snapshot.len_bytes();
        format!(
            "{}|{}",
            self.snapshot.text_for_range(0..at),
            self.snapshot.text_for_range(at..len)
        )
    }
}

impl ViewInfo for TestView {
    fn path(&self) -> &Arc<Path> {
        &self.path
    }

    fn snapshot(&self) -> &BufferSnapshot {
        &self.snapshot
    }

    fn top_row(&self) -> u32 {
        self.top_row
    }

    fn visible_rows(&self) -> u32 {
        self.visible_rows
    }

    fn is_row_hidden(&self, row: u32) -> bool {
        self.hidden.iter().any(|r| r.contains(&row))
    }

    fn tab_size(&self) -> u32 {
        self.tab_size
    }

    /// A naive scan over `()[]{}` that ignores strings and comments.
    fn matching_bracket(&self, offset: usize) -> Option<usize> {
        let text = self.snapshot.text_for_range(0..self.snapshot.len_bytes());
        let bytes = text.as_bytes();
        let pairs = [(b'(', b')'), (b'[', b']'), (b'{', b'}')];
        let here = *bytes.get(offset)?;
        if let Some(&(open, close)) = pairs.iter().find(|p| p.0 == here) {
            let mut depth = 0;
            for (i, &b) in bytes.iter().enumerate().skip(offset) {
                if b == open {
                    depth += 1;
                } else if b == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
            }
        } else if let Some(&(open, close)) = pairs.iter().find(|p| p.1 == here) {
            let mut depth = 0;
            for i in (0..=offset).rev() {
                if bytes[i] == close {
                    depth += 1;
                } else if bytes[i] == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    /// A literal search: ASCII case folded unless `case_sensitive`, and
    /// bounded by a change of char class when `whole_word`.
    fn matches(&self, query: &SearchQuery) -> Option<Arc<[Range<usize>]>> {
        if self.searching {
            return None;
        }
        let text = self.snapshot.text_for_range(0..self.snapshot.len_bytes());
        let (hay, needle) = if query.case_sensitive {
            (text.clone(), query.pattern.clone())
        } else {
            (
                text.to_ascii_lowercase(),
                query.pattern.to_ascii_lowercase(),
            )
        };
        let mut found = Vec::new();
        if needle.is_empty() {
            return Some(found.into());
        }
        let class_of = |c: char| -> CharClass { class(c, false) };
        let mut from = 0;
        while let Some(i) = hay[from..].find(&needle) {
            let range = from + i..from + i + needle.len();
            from = range.end;
            if query.whole_word {
                let first = text[range.clone()].chars().next().unwrap();
                let last = text[range.clone()].chars().next_back().unwrap();
                let before = text[..range.start].chars().next_back();
                let after = text[range.end..].chars().next();
                if before.is_some_and(|c| class_of(c) == class_of(first))
                    || after.is_some_and(|c| class_of(c) == class_of(last))
                {
                    continue;
                }
            }
            found.push(range);
        }
        Some(found.into())
    }

    fn edit_ctx(&self) -> Option<Ctx<'_>> {
        (!self.readonly).then_some(Ctx {
            snapshot: &self.snapshot,
            config: &self.config,
            language: None,
            syntax: None,
        })
    }

    /// The `nodes` that contain `range` (and are not equal to it unless it
    /// is empty), innermost first, like `SyntaxState::enclosing_nodes`.
    fn enclosing_nodes(&self, range: Range<usize>) -> Vec<Range<usize>> {
        self.node_queries.set(self.node_queries.get() + 1);
        let mut out: Vec<Range<usize>> = self
            .nodes
            .iter()
            .filter(|n| {
                n.start <= range.start && range.end <= n.end && (**n != range || range.is_empty())
            })
            .cloned()
            .collect();
        out.sort_by_key(|n| (n.end - n.start, n.start));
        out
    }
}

/// Parses `keys`: plain chars, `<C-x>`, `<Esc>`, `<CR>`, `<BS>`, `<lt>`,
/// and `<Tab>` (a tab char, which only Insert mode types).
pub(crate) fn parse_keys(keys: &str) -> Vec<Key> {
    let mut out = Vec::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            out.push(Key::Char(c));
            continue;
        }
        let name: String = chars.by_ref().take_while(|&c| c != '>').collect();
        out.push(match name.as_str() {
            "Esc" => Key::Escape,
            "CR" => Key::Enter,
            "BS" => Key::Backspace,
            "Tab" => Key::Char('\t'),
            "lt" => Key::Char('<'),
            _ => match name.strip_prefix("C-") {
                Some(rest) if rest.chars().count() == 1 => Key::Ctrl(rest.chars().next().unwrap()),
                _ => panic!("unknown key <{name}>"),
            },
        });
    }
    out
}

/// Feeds `keys` with default globals and settings; returns every outcome.
pub(crate) fn feed_keys(vim: &mut Vim, view: &TestView, keys: &str) -> Vec<Outcome> {
    let mut globals = VimGlobals::default();
    let settings = Settings::default();
    parse_keys(keys)
        .into_iter()
        .map(|k| vim.feed(k, view, &mut globals, &settings))
        .collect()
}

/// Feeds `keys` with caller-owned globals, applying `ScrollTopRow`.
pub(crate) fn feed_keys_with(
    vim: &mut Vim,
    view: &mut TestView,
    globals: &mut VimGlobals,
    keys: &str,
) -> Vec<Outcome> {
    let settings = Settings::default();
    parse_keys(keys)
        .into_iter()
        .map(|k| {
            let out = vim.feed(k, view, globals, &settings);
            if let Outcome::Handled(actions) = &out {
                for a in actions {
                    if let Action::ScrollTopRow(r) = a {
                        view.top_row = *r;
                    }
                }
            }
            out
        })
        .collect()
}

#[track_caller]
pub(crate) fn assert_keys(start: &str, keys: &str, end: &str) {
    assert_keys_with(Settings::default(), |_| {}, start, keys, end);
}

#[track_caller]
pub(crate) fn assert_keys_with(
    settings: Settings,
    view_setup: impl FnOnce(&mut TestView),
    start: &str,
    keys: &str,
    end: &str,
) {
    let (view, vim, _) = run_keys(settings, view_setup, start, keys);
    assert_eq!(
        view.marked(vim.cursor()),
        end,
        "keys {keys:?} from {start:?}"
    );
}

/// Feeds `keys` and returns the view, the vim and every emitted action.
/// Like the workbench, applies each `ScrollTopRow` to the view at once so
/// the next key sees the new top.
pub(crate) fn run_keys(
    settings: Settings,
    view_setup: impl FnOnce(&mut TestView),
    start: &str,
    keys: &str,
) -> (TestView, Vim, Vec<Action>) {
    let mut view = TestView::new(start);
    view_setup(&mut view);
    let mut vim = Vim::new(view.cursor);
    let mut globals = VimGlobals::default();
    let mut actions = Vec::new();
    for key in parse_keys(keys) {
        if let Outcome::Handled(done) = vim.feed(key, &view, &mut globals, &settings) {
            for action in done {
                if let Action::ScrollTopRow(row) = action {
                    view.top_row = row;
                }
                actions.push(action);
            }
        }
    }
    (view, vim, actions)
}

/// Feeds `keys` one at a time from `start`, applying each key's edits
/// before the next (see [`TestView::apply`]). Returns the text with `|` at
/// vim's cursor, and every emitted action.
pub(crate) fn run_edit(start: &str, keys: &str) -> (String, Vec<Action>) {
    run_edit_with(Settings::default(), |_| {}, start, keys)
}

pub(crate) fn run_edit_with(
    settings: Settings,
    view_setup: impl FnOnce(&mut TestView),
    start: &str,
    keys: &str,
) -> (String, Vec<Action>) {
    let mut view = TestView::new(start);
    view_setup(&mut view);
    let mut vim = Vim::new(view.cursor);
    let mut globals = VimGlobals::default();
    let mut actions = Vec::new();
    let mut run = Run {
        view: &mut view,
        vim: &mut vim,
        globals: &mut globals,
        settings: &settings,
        actions: &mut actions,
        search: None,
        cmdline: None,
    };
    for key in parse_keys(keys) {
        run.key(key, false);
    }
    (view.marked(vim.cursor()), actions)
}

/// What [`run_edit_with`] drives.
struct Run<'a> {
    view: &'a mut TestView,
    vim: &'a mut Vim,
    globals: &'a mut VimGlobals,
    settings: &'a Settings,
    actions: &'a mut Vec<Action>,
    /// The open search: its direction, the pattern typed so far, and
    /// vim's cursor when it opened.
    search: Option<(bool, String, Point)>,
    /// The open `:` command line: its text so far.
    cmdline: Option<String>,
}

/// The literal, case-insensitive query the test search submits.
pub(crate) fn literal(pattern: &str) -> SearchQuery {
    SearchQuery {
        pattern: pattern.into(),
        is_regex: false,
        case_sensitive: false,
        whole_word: false,
    }
}

impl Run<'_> {
    /// One key: typed in Insert mode (recorded unless `replaying`), else
    /// fed to vim with its actions applied and its replay run.
    fn key(&mut self, key: Key, replaying: bool) {
        if let Some((backward, pattern, origin)) = &mut self.search {
            match key {
                Key::Char(c) => pattern.push(c),
                Key::Backspace => {
                    pattern.pop();
                }
                Key::Enter => {
                    let (backward, pattern, origin) = (*backward, literal(pattern), *origin);
                    self.search = None;
                    self.submit(pattern, backward, origin);
                }
                Key::Escape => {
                    let origin = *origin;
                    self.search = None;
                    self.vim.search_dismissed();
                    self.vim.set_cursor(origin);
                    self.vim.clamp(&self.view.snapshot);
                }
                Key::Ctrl(_) => {}
            }
            return;
        }
        if let Some(text) = &mut self.cmdline {
            match key {
                Key::Char(c) => text.push(c),
                Key::Backspace => {
                    text.pop();
                }
                Key::Enter => {
                    let line = self.cmdline.take().unwrap_or_default();
                    let from = self.vim.cursor();
                    let out = self
                        .vim
                        .ex_command(&line, self.view, self.globals, self.settings);
                    self.outcome(out, false, from);
                }
                Key::Escape => self.cmdline = None,
                Key::Ctrl(_) => {}
            }
            return;
        }
        let insert = self.vim.mode() == crate::Mode::Insert;
        let typed = match key {
            Key::Char('\t') => Some(Typed::Tab),
            Key::Char(c) => Some(Typed::Text(c.to_string())),
            Key::Enter => Some(Typed::Newline),
            Key::Backspace => Some(Typed::DeleteLeft),
            _ => None,
        };
        if insert && let Some(typed) = typed {
            self.view.type_typed(self.vim, &typed);
            if !replaying {
                self.vim.record_typed(typed);
            }
            return;
        }
        let from = self.vim.cursor();
        let out = self.vim.feed(key, self.view, self.globals, self.settings);
        self.outcome(out, insert, from);
    }

    /// Submits the search `query` from `origin`, as the workbench does on
    /// Enter in the find widget or for a replay's search step.
    fn submit(&mut self, query: SearchQuery, backward: bool, origin: Point) {
        let out = self.vim.search_submitted(
            query,
            backward,
            origin,
            self.view,
            self.globals,
            self.settings,
        );
        self.outcome(out, false, origin);
    }

    /// Carries out what a key or a submitted search returned; `insert`:
    /// vim was in Insert mode before, with its cursor at `from`.
    fn outcome(&mut self, out: Outcome, insert: bool, from: Point) {
        match out {
            Outcome::Handled(done) => {
                let entered = !insert && self.vim.mode() == crate::Mode::Insert;
                if entered {
                    self.view.begin_group(from);
                }
                self.view.apply(self.vim, &done);
                for a in &done {
                    match a {
                        Action::OpenSearch { backward } => {
                            self.search = Some((*backward, String::new(), self.vim.cursor()));
                        }
                        Action::OpenCommandLine { prefill } => {
                            self.cmdline = Some(prefill.clone());
                        }
                        _ => {}
                    }
                }
                self.actions.extend(done);
                if insert && self.vim.mode() != crate::Mode::Insert {
                    self.view.end_group();
                }
            }
            Outcome::Replay(steps) => {
                self.view.begin_group(from);
                for step in steps {
                    match step {
                        ReplayStep::Key(k) => self.key(k, true),
                        // The command that should have entered Insert
                        // failed: the replay stops.
                        ReplayStep::Type(_) if self.vim.mode() != crate::Mode::Insert => break,
                        ReplayStep::Type(t) => self.view.type_typed(self.vim, &t),
                        // As in the workbench: no operator, no search.
                        ReplayStep::Search { .. } if !self.vim.operator_pending() => break,
                        ReplayStep::Search {
                            pattern,
                            backward,
                            is_regex,
                            case_sensitive,
                            whole_word,
                        } => {
                            let query = SearchQuery {
                                pattern,
                                is_regex,
                                case_sensitive,
                                whole_word,
                            };
                            let origin = self.vim.cursor();
                            self.submit(query, backward, origin);
                        }
                    }
                }
                self.view.end_group();
            }
            Outcome::Substitute(job) => {
                let result = job.run(&self.view.snapshot);
                let out = self.vim.finish_substitute(
                    &job,
                    result,
                    self.view,
                    self.globals,
                    self.settings,
                );
                self.outcome(out, false, from);
            }
            Outcome::Unhandled | Outcome::Retry(_) => {}
        }
    }
}

#[track_caller]
pub(crate) fn assert_edit(start: &str, keys: &str, end: &str) {
    assert_edit_with(Settings::default(), |_| {}, start, keys, end);
}

#[track_caller]
pub(crate) fn assert_edit_with(
    settings: Settings,
    view_setup: impl FnOnce(&mut TestView),
    start: &str,
    keys: &str,
    end: &str,
) {
    let (text, _) = run_edit_with(settings, view_setup, start, keys);
    assert_eq!(text, end, "keys {keys:?} from {start:?}");
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lucerna_editor::{EditKind, EditPlan, SelectionSet};

    use super::*;

    /// V1 keys emit no edits: `run_edit` ends where `assert_keys` does.
    #[test]
    fn run_edit_matches_navigation() {
        for (start, keys, end) in [
            ("a|bc def", "w", "abc |def"),
            ("abc\nd|ef", "kQ<Esc>", "a|bc\ndef"),
            ("one |two", "vey<Esc>", "one |two"),
            ("a<b|", "F<lt>", "a|<b"),
        ] {
            assert_keys(start, keys, end);
            assert_edit(start, keys, end);
            let (_, actions) = run_edit(start, keys);
            assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        }
    }

    #[test]
    fn parses_backspace() {
        assert_eq!(parse_keys("a<BS>"), [Key::Char('a'), Key::Backspace]);
    }

    #[test]
    fn edits_apply_then_clamp_and_undo_redo() {
        let mut view = TestView::new("abc\nde|f");
        let mut vim = Vim::new(view.cursor);
        // Delete "def" and the line break before it.
        let plan = EditPlan::new(
            vec![(3..7, Arc::from(""))],
            SelectionSet::cursor(2),
            EditKind::Other,
        );
        view.apply(&mut vim, &[Action::Edit(plan)]);
        assert_eq!(view.marked(vim.cursor()), "ab|c");
        view.apply(&mut vim, &[Action::Undo(1)]);
        assert_eq!(view.marked(vim.cursor()), "abc\nde|f");
        view.apply(&mut vim, &[Action::Redo(1)]);
        assert_eq!(view.marked(vim.cursor()), "ab|c");
        // Nothing left to redo; undo past the start stays at the start.
        view.apply(&mut vim, &[Action::Redo(1), Action::Undo(5)]);
        assert_eq!(view.marked(vim.cursor()), "abc\nde|f");
    }

    #[test]
    fn applies_several_edits_in_one_plan() {
        let mut view = TestView::new("abcd|ef");
        let mut vim = Vim::new(view.cursor);
        let plan = EditPlan::new(
            vec![(0..1, Arc::from("XY")), (4..5, Arc::from(""))],
            SelectionSet::cursor(5),
            EditKind::Other,
        );
        view.apply(&mut vim, &[Action::Edit(plan)]);
        // Vim keeps its own cursor (offset 4), clamped to the new text.
        assert_eq!(view.marked(vim.cursor()), "XYbc|df");
    }

    #[test]
    fn readonly_view_has_no_edit_ctx() {
        let mut view = TestView::new("|abc");
        assert!(view.edit_ctx().is_some());
        view.readonly = true;
        assert!(view.edit_ctx().is_none());
    }
}
