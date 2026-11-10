//! String-based test helpers: `|` marks the cursor.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use lucerna_text::{Buffer, BufferSnapshot, Point};

use crate::classify::{CharClass, class};
use crate::{Key, Outcome, SearchQuery, Settings, ViewInfo, Vim, VimGlobals};

pub(crate) struct TestView {
    pub path: Arc<Path>,
    pub snapshot: BufferSnapshot,
    pub cursor: Point,
    pub top_row: u32,
    pub visible_rows: u32,
    /// Rows inside closed folds.
    pub hidden: Vec<Range<u32>>,
    pub tab_size: u32,
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
        }
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
}

/// Parses `keys`: plain chars, `<C-x>`, `<Esc>`, `<CR>`.
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
    let mut view = TestView::new(start);
    view_setup(&mut view);
    let mut vim = Vim::new(view.cursor);
    let mut globals = VimGlobals::default();
    for key in parse_keys(keys) {
        vim.feed(key, &view, &mut globals, &settings);
    }
    assert_eq!(
        view.marked(vim.cursor()),
        end,
        "keys {keys:?} from {start:?}"
    );
}
