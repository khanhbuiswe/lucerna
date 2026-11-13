//! Text objects (spec §5): the ranges `iw`, `a(`, `i"`, `it`, `ap`, `is`
//! and the rest cover, after an operator or in Visual mode.

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

use crate::ViewInfo;
use crate::classify::{CharClass, class, line};
use crate::operator::{MotionKind, OpRange, row_start};
use crate::visual::{line_end, line_len, next_row_start};

/// How far from the cursor bracket and tag objects look, in bytes.
const SCAN_WINDOW: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Object {
    Word,
    BigWord,
    Paren,
    Brace,
    Bracket,
    Angle,
    DoubleQuote,
    SingleQuote,
    BackQuote,
    Tag,
    Paragraph,
    Sentence,
}

impl Object {
    /// The object a key after `i` or `a` names.
    pub(crate) fn from_key(c: char) -> Option<Object> {
        Some(match c {
            'w' => Object::Word,
            'W' => Object::BigWord,
            '(' | ')' | 'b' => Object::Paren,
            '{' | '}' | 'B' => Object::Brace,
            '[' | ']' => Object::Bracket,
            '<' | '>' => Object::Angle,
            '"' => Object::DoubleQuote,
            '\'' => Object::SingleQuote,
            '`' => Object::BackQuote,
            't' => Object::Tag,
            'p' => Object::Paragraph,
            's' => Object::Sentence,
            _ => return None,
        })
    }

    /// Bracket and tag objects nest: a larger count reaches an outer one.
    pub(crate) fn nests(self) -> bool {
        matches!(
            self,
            Object::Paren | Object::Brace | Object::Bracket | Object::Angle | Object::Tag
        )
    }
}

/// The range `obj` covers at `cursor`; `around` is `a…`, else `i…`.
///
/// Paragraphs, and the inner part of a bracket pair that stands on rows of
/// its own, are linewise (from the start of the first row to the end of
/// the last row's text); every other object is the exact bytes, which may
/// be empty (`i(` on `()`). `None` when there is no such object.
pub(crate) fn object_range(
    s: &BufferSnapshot,
    cursor: Point,
    obj: Object,
    around: bool,
    count: u32,
    view: &dyn ViewInfo,
) -> Option<OpRange> {
    let count = count.max(1);
    let chars = |range: Range<usize>| OpRange {
        range,
        kind: MotionKind::Exclusive,
    };
    match obj {
        Object::Word | Object::BigWord => {
            word(s, cursor, obj == Object::BigWord, around, count).map(chars)
        }
        Object::Paren => bracket(s, cursor, (b'(', b')'), around, count, view),
        Object::Brace => bracket(s, cursor, (b'{', b'}'), around, count, view),
        Object::Bracket => bracket(s, cursor, (b'[', b']'), around, count, view),
        Object::Angle => bracket(s, cursor, (b'<', b'>'), around, count, view),
        Object::DoubleQuote => quote(s, cursor, '"', around, count).map(chars),
        Object::SingleQuote => quote(s, cursor, '\'', around, count).map(chars),
        Object::BackQuote => quote(s, cursor, '`', around, count).map(chars),
        Object::Tag => tag(s, cursor, around, count, view).map(chars),
        Object::Sentence => sentence(s, cursor, around, count).map(chars),
        Object::Paragraph => {
            let (first, last) = paragraph(s, cursor.row, around, count)?;
            Some(OpRange {
                range: row_start(s, first)..line_end(s, last),
                kind: MotionKind::Linewise,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Words

/// What a run of the text around the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    /// Chars of one class: a word, or a run of punctuation (`iskeyword`).
    Text,
    /// Spaces and tabs within one row.
    Blank,
    /// A line break.
    Break,
}

/// `text` cut into runs, each a byte range and its kind.
fn runs(text: &str, big: bool) -> Vec<(Range<usize>, Run)> {
    let mut out: Vec<(Range<usize>, Run)> = Vec::new();
    let mut prev: Option<(Run, CharClass)> = None;
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        let here = match c {
            '\n' => (Run::Break, CharClass::Space),
            // `\r` before `\n` belongs to the break.
            '\r' if text[end..].starts_with('\n') => (Run::Break, CharClass::Space),
            c if c.is_whitespace() => (Run::Blank, CharClass::Space),
            c => (Run::Text, class(c, big)),
        };
        let joins = prev == Some(here) && here.0 != Run::Break
            || (here.0 == Run::Break && c == '\n' && text[..i].ends_with('\r'));
        match out.last_mut() {
            Some(last) if joins => last.0.end = end,
            _ => out.push((i..end, here.0)),
        }
        prev = Some(here);
    }
    out
}

/// `iw` `aw` `iW` `aW` (`:h iw`, `:h aw`). The text is a sequence of runs:
/// words (a word or a run of punctuation), blanks within a row, and line
/// breaks.
/// - `iw` takes `count` runs from the one under the cursor, blanks
///   counting as words and line breaks passed over.
/// - `aw` on a word takes `count` words, each with the blanks after it on
///   its row. When the last has none, the blanks before the first join,
///   unless they are the row's indent.
/// - `aw` on blanks takes them and the `count` words after them.
/// - On an empty row both give an empty range.
fn word(
    s: &BufferSnapshot,
    cursor: Point,
    big: bool,
    around: bool,
    count: u32,
) -> Option<Range<usize>> {
    let base = row_start(s, cursor.row);
    if line_len(s, cursor.row) == 0 {
        return Some(base..base);
    }
    // Every unit lies within a few rows, past blank-only ones.
    let last_row = s.line_count().saturating_sub(1);
    let rows = count.saturating_mul(2).saturating_add(64);
    let end_row = cursor.row.saturating_add(rows).min(last_row);
    let text = s.text_for_range(base..next_row_start(s, end_row));
    let runs = runs(&text, big);
    let cur = s.point_to_offset(cursor) - base;
    let k = runs.iter().position(|(r, _)| cur < r.end)?;
    let count = count as usize;
    // The indices of the runs after `k` (and `k` itself) that are not
    // line breaks, or only words.
    let from_k = |only_text: bool| {
        runs.iter()
            .enumerate()
            .skip(k)
            .filter(move |(_, (_, kind))| {
                if only_text {
                    *kind == Run::Text
                } else {
                    *kind != Run::Break
                }
            })
            .map(|(i, _)| i)
    };
    let (start, end) = if !around {
        let last = from_k(false).nth(count - 1)?;
        (runs[k].0.start, runs[last].0.end)
    } else if runs[k].1 != Run::Text {
        let last = from_k(true).nth(count - 1)?;
        (runs[k].0.start, runs[last].0.end)
    } else {
        let last = from_k(true).nth(count - 1)?;
        match runs.get(last + 1) {
            Some((blank, Run::Blank)) => (runs[k].0.start, blank.end),
            _ => {
                let before = k
                    .checked_sub(1)
                    .filter(|&b| runs[b].1 == Run::Blank && b > 0 && runs[b - 1].1 != Run::Break);
                let start = before.map_or(runs[k].0.start, |b| runs[b].0.start);
                (start, runs[last].0.end)
            }
        }
    };
    Some(base + start..base + end)
}

// ---------------------------------------------------------------------------
// Brackets

/// Whether the node starting at `start` reads as a string or a comment. A
/// node here is only a byte range, so its text decides:
/// - a string starts with a quote, after at most two ASCII letters (`r"`,
///   `b'`, `f"`, `rb"`) and any `#`s (`r#"`), and ends with that quote,
///   perhaps followed by `#`s (a Rust raw string `r#"…"#`). Requiring the closing quote keeps out nodes that
///   merely start with a quote, such as a loop with a `'label:`;
/// - a comment starts with `//`, `/*`, `--` or `#`, except `#[` and `#!`
///   (Rust attributes).
fn is_string_or_comment(s: &BufferSnapshot, node: &Range<usize>) -> bool {
    let len = s.len_bytes();
    if node.start >= node.end || node.end > len {
        return false;
    }
    let at = |i: usize| (i < node.end).then(|| s.byte_at(i));
    let (a, b) = (at(node.start), at(node.start + 1));
    match (a, b) {
        (Some(b'/'), Some(b'/' | b'*')) | (Some(b'-'), Some(b'-')) => return true,
        (Some(b'#'), Some(b'[' | b'!')) => return false,
        (Some(b'#'), _) => return true,
        _ => {}
    }
    let mut i = node.start;
    while i < node.start + 2 && at(i).is_some_and(|c| c.is_ascii_alphabetic()) {
        i += 1;
    }
    while at(i) == Some(b'#') {
        i += 1;
    }
    let Some(q @ (b'"' | b'\'' | b'`')) = at(i) else {
        return false;
    };
    let mut end = node.end - 1;
    while end > i && s.byte_at(end) == b'#' {
        end -= 1;
    }
    end > i && s.byte_at(end) == q
}

/// The brackets of one kind in a window of the text, with the syntax
/// tree's strings and comments to skip.
struct Brackets<'a> {
    s: &'a BufferSnapshot,
    view: &'a dyn ViewInfo,
    /// The window's first offset.
    lo: usize,
    bytes: Vec<u8>,
    pair: (u8, u8),
    /// The cursor's offset: a string or comment holding it is searched
    /// like plain text.
    cursor: usize,
    /// The strings and comments found so far, so a scan that comes back
    /// over one skips it without asking the tree again.
    found: std::cell::RefCell<Vec<Range<usize>>>,
    /// Tree queries made; past [`MAX_NODE_QUERIES`] the scan is text-only.
    queries: std::cell::Cell<u32>,
}

/// The most syntax-tree queries one bracket object makes. Each query walks
/// the tree, so a scan over thousands of brackets stops asking and treats
/// the rest as plain text.
const MAX_NODE_QUERIES: u32 = 2_000;

impl Brackets<'_> {
    /// The string or comment node the bracket at window index `i` lies
    /// strictly inside, unless that node also holds the cursor.
    fn skipped(&self, i: usize) -> Option<Range<usize>> {
        let at = self.lo + i;
        let skips = |n: &Range<usize>| {
            n.start < at && at < n.end && !(n.start <= self.cursor && self.cursor < n.end)
        };
        if let Some(n) = self.found.borrow().iter().find(|n| skips(n)) {
            return Some(n.clone());
        }
        if self.queries.get() >= MAX_NODE_QUERIES {
            return None;
        }
        self.queries.set(self.queries.get() + 1);
        let n = self
            .view
            .enclosing_nodes(at..at + 1)
            .into_iter()
            .find(|n| skips(n) && is_string_or_comment(self.s, n))?;
        self.found.borrow_mut().push(n.clone());
        Some(n)
    }

    /// The unmatched opening bracket at or before window index `from`.
    fn open_before(&self, from: usize) -> Option<usize> {
        let (open, close) = self.pair;
        let mut depth = 0u32;
        let mut i = from.min(self.bytes.len().checked_sub(1)?) as isize;
        while i >= 0 {
            let b = self.bytes[i as usize];
            if b == open || b == close {
                if let Some(n) = self.skipped(i as usize) {
                    i = n.start as isize - self.lo as isize - 1;
                    continue;
                }
                if b == close {
                    depth += 1;
                } else if depth == 0 {
                    return Some(i as usize);
                } else {
                    depth -= 1;
                }
            }
            i -= 1;
        }
        None
    }

    /// The unmatched closing bracket at or after window index `from`.
    fn close_after(&self, from: usize) -> Option<usize> {
        let (open, close) = self.pair;
        let mut depth = 0u32;
        let mut i = from;
        while i < self.bytes.len() {
            let b = self.bytes[i];
            if b == open || b == close {
                if let Some(n) = self.skipped(i) {
                    i = n.end.saturating_sub(self.lo).max(i + 1);
                    continue;
                }
                if b == open {
                    depth += 1;
                } else if depth == 0 {
                    return Some(i);
                } else {
                    depth -= 1;
                }
            }
            i += 1;
        }
        None
    }

    /// The pair enclosing window index `cur`, `count` levels out; a bracket
    /// under the cursor counts as enclosing it.
    fn enclosing(&self, cur: usize, count: u32) -> Option<(usize, usize)> {
        let on_open = self.bytes.get(cur) == Some(&self.pair.0) && self.skipped(cur).is_none();
        let mut open = if on_open {
            cur
        } else {
            self.open_before(cur.checked_sub(1)?)?
        };
        for _ in 1..count {
            open = self.open_before(open.checked_sub(1)?)?;
        }
        Some((open, self.close_after(open + 1)?))
    }

    /// The pair of the first opening bracket at or after `cur`.
    fn next_pair(&self, cur: usize) -> Option<(usize, usize)> {
        let mut i = cur;
        while i < self.bytes.len() {
            if self.bytes[i] == self.pair.0 {
                if let Some(n) = self.skipped(i) {
                    i = n.end.saturating_sub(self.lo).max(i + 1);
                    continue;
                }
                // An unclosed bracket ends the search: trying every later
                // one could take quadratic time.
                return self.close_after(i + 1).map(|close| (i, close));
            }
            i += 1;
        }
        None
    }
}

/// `i(` `a(` and the other bracket pairs: the pair around the cursor,
/// `count` levels out, else the next pair after it (as VSCodeVim does).
///
/// The inner part follows Vim on rows: when the opening bracket ends its
/// row, the part starts at the next row's start (the line break is not
/// taken); when, in addition, the closing bracket has only white space
/// before it on its row, the part is the rows in between, linewise, so
/// `di{` leaves `{\n}` and `ci{` works like `cc`.
fn bracket(
    s: &BufferSnapshot,
    cursor: Point,
    pair: (u8, u8),
    around: bool,
    count: u32,
    view: &dyn ViewInfo,
) -> Option<OpRange> {
    let at = s.point_to_offset(cursor);
    let lo = s.floor_char_boundary(at.saturating_sub(SCAN_WINDOW));
    let hi = s.floor_char_boundary(at.saturating_add(SCAN_WINDOW));
    let brackets = Brackets {
        s,
        view,
        lo,
        bytes: s.text_for_range(lo..hi).into_bytes(),
        pair,
        cursor: at,
        found: Default::default(),
        queries: Default::default(),
    };
    let cur = at - lo;
    let (open, close) = brackets
        .enclosing(cur, count)
        .or_else(|| brackets.next_pair(cur))?;
    let chars = |range: Range<usize>| {
        Some(OpRange {
            range,
            kind: MotionKind::Exclusive,
        })
    };
    if around {
        return chars(lo + open..lo + close + 1);
    }
    let (open, close) = (lo + open, lo + close);
    let open_row = s.offset_to_point(open).row;
    let close_at = s.offset_to_point(close);
    let blank = |r: Range<usize>| s.text_for_range(r).trim().is_empty();
    let opener_ends_row = close_at.row > open_row && blank(open + 1..line_end(s, open_row));
    if !opener_ends_row {
        return chars(open + 1..close);
    }
    let first = open_row + 1;
    let closer_starts_row = blank(row_start(s, close_at.row)..close);
    if closer_starts_row && close_at.row > first {
        return Some(OpRange {
            range: row_start(s, first)..line_end(s, close_at.row - 1),
            kind: MotionKind::Linewise,
        });
    }
    if closer_starts_row {
        // `{\n}`: nothing in between.
        return chars(open + 1..open + 1);
    }
    chars(row_start(s, first)..close)
}

// ---------------------------------------------------------------------------
// Quotes

/// Whether the char at byte `i` of `text` follows an odd run of `\`.
fn escaped(text: &str, i: usize) -> bool {
    text.as_bytes()[..i]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

/// `i"` `a"` and the other quotes, on the cursor's row only. Quotes pair
/// up counting from the row's start, skipping escaped ones; a cursor
/// before or between pairs takes the next pair, and after the last one
/// fails. `a"` takes the white space after, or before when there is none
/// after. `i"` with a count of 2 or more takes the quotes too.
fn quote(
    s: &BufferSnapshot,
    cursor: Point,
    q: char,
    around: bool,
    count: u32,
) -> Option<Range<usize>> {
    let text = line(s, cursor.row);
    let col = cursor.column as usize;
    let quotes: Vec<usize> = text
        .char_indices()
        .filter(|&(i, c)| c == q && !escaped(&text, i))
        .map(|(i, _)| i)
        .collect();
    let first = match quotes.iter().position(|&i| i == col) {
        Some(k) => k - k % 2,
        None => {
            let before = quotes.iter().filter(|&&i| i < col).count();
            // Odd: inside the pair the last quote opens; even: the next.
            before - before % 2
        }
    };
    let (open, close) = (*quotes.get(first)?, *quotes.get(first + 1)?);
    let base = row_start(s, cursor.row);
    let (mut from, mut to) = if around || count >= 2 {
        (open, close + 1)
    } else {
        (open + 1, close)
    };
    if around {
        let white = |c: char| c == ' ' || c == '\t';
        let after = text[to..].len() - text[to..].trim_start_matches(white).len();
        if after > 0 {
            to += after;
        } else {
            from -= text[..from].len() - text[..from].trim_end_matches(white).len();
        }
    }
    Some(base + from..base + to)
}

// ---------------------------------------------------------------------------
// Tags

/// The byte length of the tag name at the start of `rest`.
fn tag_name_len(rest: &[u8]) -> usize {
    if !rest.first().is_some_and(|b| b.is_ascii_alphabetic()) {
        return 0;
    }
    rest.iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        .count()
}

/// The index after the `>` that ends the tag starting at `from`, skipping
/// quoted attribute values.
fn tag_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut quote = None;
    for (i, &b) in bytes.iter().enumerate().skip(from) {
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => return Some(i + 1),
            (None, _) => {}
        }
    }
    None
}

/// An element: its opening tag's range and its closing tag's range.
type Element = (Range<usize>, Range<usize>);

/// The most bytes read from the start of an element node for its opening
/// tag, and from its end for its closing tag.
const TAG_HEAD: usize = 4096;
const TAG_TAIL: usize = 256;

/// The element a syntax node covers, if its text is `<name …>…</name>`.
/// Only the node's head and tail are read, and nodes larger than
/// [`SCAN_WINDOW`] are passed over, so ancestors up to the root cost little.
fn element_node(s: &BufferSnapshot, node: &Range<usize>) -> Option<Element> {
    // A tree a parse behind the text may hold ranges that no longer fit.
    let fits = |at: usize| at <= s.len_bytes() && s.floor_char_boundary(at) == at;
    if node.start >= node.end
        || node.end - node.start > SCAN_WINDOW
        || !fits(node.start)
        || !fits(node.end)
    {
        return None;
    }
    let head_end = s.floor_char_boundary((node.start + TAG_HEAD).min(node.end));
    let head = s.text_for_range(node.start..head_end).into_bytes();
    let name = tag_name_len(head.get(1..)?);
    if head[0] != b'<' || name == 0 {
        return None;
    }
    let open_end = node.start + tag_end(&head, 1 + name)?;
    let name = &head[1..1 + name];
    let tail_start = s.floor_char_boundary(node.end.saturating_sub(TAG_TAIL).max(node.start));
    let tail = s.text_for_range(tail_start..node.end).into_bytes();
    let body = tail.strip_suffix(b">")?.trim_ascii_end();
    let close_start = tail_start + body.strip_suffix(name)?.strip_suffix(b"</")?.len();
    (close_start >= open_end).then_some((node.start..open_end, close_start..node.end))
}

/// Every element in `bytes`, from a scan that pairs opening and closing
/// tags by name. A closing tag closes the latest open one of its name
/// (dropping the unclosed ones inside it); self-closing tags, comments
/// and declarations are skipped.
fn scan_elements(bytes: &[u8]) -> Vec<Element> {
    let mut open: Vec<(Range<usize>, &[u8])> = Vec::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &bytes[i + 1..];
        if rest.starts_with(b"!--") {
            i = find(bytes, i + 4, b"-->").map_or(bytes.len(), |e| e + 3);
            continue;
        }
        let closing = rest.first() == Some(&b'/');
        let name_at = i + 1 + usize::from(closing);
        let name = tag_name_len(&bytes[name_at..]);
        if name == 0 {
            i += 1;
            continue;
        }
        let Some(end) = tag_end(bytes, name_at + name) else {
            break;
        };
        let name = &bytes[name_at..name_at + name];
        if closing {
            if let Some(k) = open.iter().rposition(|(_, n)| *n == name) {
                out.push((open[k].0.clone(), i..end));
                open.truncate(k);
            }
        } else if bytes[end - 2] != b'/' {
            open.push((i..end, name));
        }
        i = end;
    }
    out
}

/// The first index of `needle` in `bytes` at or after `from`.
fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| from + i)
}

/// `it` `at`: the element around the cursor, `count` levels out. With a
/// syntax tree, the nodes around the cursor whose text is a whole element;
/// without one (or when none is), a scan of the text for tags.
fn tag(
    s: &BufferSnapshot,
    cursor: Point,
    around: bool,
    count: u32,
    view: &dyn ViewInfo,
) -> Option<Range<usize>> {
    let at = s.point_to_offset(cursor);
    let mut elements: Vec<Element> = view
        .enclosing_nodes(at..at)
        .iter()
        .filter_map(|n| element_node(s, n))
        .collect();
    if elements.is_empty() {
        let lo = s.floor_char_boundary(at.saturating_sub(SCAN_WINDOW));
        let hi = s.floor_char_boundary(at.saturating_add(SCAN_WINDOW));
        let bytes = s.text_for_range(lo..hi).into_bytes();
        elements = scan_elements(&bytes)
            .into_iter()
            .map(|(a, b)| (a.start + lo..a.end + lo, b.start + lo..b.end + lo))
            .filter(|(open, close)| open.start <= at && at < close.end)
            .collect();
        // Innermost first.
        elements.sort_by_key(|(open, _)| std::cmp::Reverse(open.start));
    }
    let (open, close) = elements.into_iter().nth(count as usize - 1)?;
    Some(if around {
        open.start..close.end
    } else {
        open.end..close.start
    })
}

// ---------------------------------------------------------------------------
// Paragraphs and sentences

fn is_blank(s: &BufferSnapshot, row: u32) -> bool {
    line(s, row).chars().all(char::is_whitespace)
}

/// The rows of the run of blank, or of non-blank, rows around `row`.
fn block(s: &BufferSnapshot, row: u32) -> (u32, u32) {
    let last = s.line_count().saturating_sub(1);
    let blank = is_blank(s, row);
    let (mut first, mut end) = (row, row);
    while first > 0 && is_blank(s, first - 1) == blank {
        first -= 1;
    }
    while end < last && is_blank(s, end + 1) == blank {
        end += 1;
    }
    (first, end)
}

/// `ip` `ap` as rows. A paragraph is a run of non-blank rows or a run of
/// blank ones; `ip` takes `count` such runs. `ap` takes a paragraph and
/// the blank rows after it (on blank rows: those and the paragraph after
/// them), `count` times; when no blank rows follow, the ones before it.
fn paragraph(s: &BufferSnapshot, row: u32, around: bool, count: u32) -> Option<(u32, u32)> {
    let last = s.line_count().saturating_sub(1);
    let (mut first, mut end) = block(s, row);
    if !around {
        for _ in 1..count {
            if end >= last {
                return None;
            }
            end = block(s, end + 1).1;
        }
        return Some((first, end));
    }
    let on_blank = is_blank(s, row);
    let mut trailing_blank = false;
    for k in 0..count {
        if k > 0 {
            if end >= last {
                return None;
            }
            end = block(s, end + 1).1;
        }
        // The second half of the unit: blank rows after a paragraph, or
        // the paragraph after blank rows.
        if end < last {
            end = block(s, end + 1).1;
            trailing_blank = !on_blank;
        } else {
            trailing_blank = on_blank;
        }
    }
    if !trailing_blank && !on_blank && first > 0 && is_blank(s, first - 1) {
        first = block(s, first - 1).0;
    }
    Some((first, end))
}

/// `is` `as`. A sentence ends at `.`, `!` or `?` (then perhaps closing
/// `)`, `]`, `"` or `'`) followed by white space or the paragraph's end;
/// paragraphs bound sentences. The text splits into sentences and the
/// white space between them: `is` takes `count` of those parts from the
/// cursor's; `as` takes `count` sentences with the white space after each
/// (before the first, when the last has none after it), or from white
/// space, the white space and the sentences after it.
fn sentence(s: &BufferSnapshot, cursor: Point, around: bool, count: u32) -> Option<Range<usize>> {
    if is_blank(s, cursor.row) {
        return None;
    }
    let (first, last) = block(s, cursor.row);
    let base = row_start(s, first);
    let text = s.text_for_range(base..line_end(s, last));
    let cur = s.point_to_offset(cursor) - base;
    let parts = sentence_parts(&text);
    let k = parts
        .iter()
        .position(|(r, _)| cur < r.end)
        .unwrap_or(parts.len().checked_sub(1)?);
    let count = count as usize;
    let (from, to) = if !around {
        let end = parts.get(k + count - 1)?;
        (parts[k].0.start, end.0.end)
    } else if parts[k].1 {
        // From white space: it and `count` sentences after it.
        (parts[k].0.start, parts.get(k + 2 * count - 1)?.0.end)
    } else {
        let last_sentence = parts.get(k + 2 * (count - 1))?;
        match parts.get(k + 2 * count - 1) {
            Some(white) => (parts[k].0.start, white.0.end),
            None => {
                let from = match k.checked_sub(1).map(|i| &parts[i]) {
                    Some((white, true)) => white.start,
                    _ => parts[k].0.start,
                };
                (from, last_sentence.0.end)
            }
        }
    };
    Some(base + from..base + to)
}

/// `text` split into sentences and the white space between them (`true`
/// for white space), in order and covering it all.
fn sentence_parts(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut parts = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut start = 0;
    let mut in_white = text.starts_with(char::is_whitespace);
    while let Some((i, c)) = chars.next() {
        if in_white {
            if !c.is_whitespace() {
                parts.push((start..i, true));
                start = i;
                in_white = false;
            } else {
                continue;
            }
        }
        if matches!(c, '.' | '!' | '?') {
            let mut end = i + 1;
            while let Some(&(j, d)) = chars.peek() {
                if !matches!(d, ')' | ']' | '"' | '\'') {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            if chars.peek().is_none_or(|&(_, d)| d.is_whitespace()) {
                parts.push((start..end, false));
                start = end;
                in_white = true;
            }
        }
    }
    if start < text.len() {
        parts.push((start..text.len(), in_white));
    }
    parts
}

#[cfg(test)]
mod tests {
    use lucerna_text::Point;

    use crate::test_util::{assert_edit, assert_edit_with, run_edit, run_edit_with, run_keys};
    use crate::{Action, Mode, Settings, VimSelection};

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'cib' between sets of parentheses"
    #[test]
    fn cib_between_parentheses() {
        assert_edit(
            "one (two) th|ree (four) five",
            "cib",
            "one (two) three (|) five",
        );
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle count prefixed 'ci)'"
    #[test]
    fn count_prefixed_ci_paren() {
        assert_edit(" b(l(baz(f|oo)baz)a)h ", "c3i)", " b(|)h ");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'ci\"' starting behind the quoted word"
    #[test]
    fn ci_quote_behind_the_quoted_word() {
        assert_edit("|one \"two\"", "ci\"", "one \"|\"");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'ca\"' starting inside the quoted word"
    #[test]
    fn ca_quote_takes_trailing_space() {
        assert_edit("one \"t|wo\" three", "ca\"", "one |three");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'c2i\"' starting inside the quoted word"
    #[test]
    fn c2i_quote_takes_the_quotes() {
        assert_edit("one \"t|wo\" \nthree", "c2i\"", "one | \nthree");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'daw' on word with leading spaces"
    #[test]
    fn daw_takes_leading_space_without_trailing() {
        assert_edit("one two th|ree, four ", "daw", "one two|, four ");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'd2aw' on word at end of line"
    #[test]
    fn d2aw_across_lines() {
        assert_edit(
            "one two three, fo|ur \nfive six",
            "d2aw",
            "one two three, |six",
        );
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'cit' on a matching tag"
    #[test]
    fn cit_on_matching_tag() {
        assert_edit("<blink>he|llo</blink>", "cit", "<blink>|</blink>");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'cat' on a matching tag"
    #[test]
    fn cat_on_matching_tag() {
        assert_edit("one <blink>he|llo</blink> two", "cat", "one | two");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'dip' on empty lines"
    #[test]
    fn dip_on_empty_lines() {
        assert_edit("foo\n\n|\n\nfun", "dip", "foo\n|fun");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'dap' on paragraph"
    #[test]
    fn dap_takes_trailing_blank_lines() {
        assert_edit("foo\n\nbar baz\nbar |baz\n\nfun", "dap", "foo\n\n|fun");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'das' on the first sentence"
    #[test]
    fn das_first_sentence() {
        assert_edit(
            "That's my sec|ret, Captain. I'm always angry.",
            "das",
            "|I'm always angry.",
        );
    }

    #[test]
    fn ciw_on_multibyte_word() {
        assert_edit("Xin ch|ào", "ciw", "Xin |");
        let (_, actions) = run_edit("Xin ch|ào", "ciw");
        assert!(actions.contains(&Action::ModeChanged(Mode::Insert)));
    }

    #[test]
    fn diw_on_whitespace_takes_the_run() {
        assert_edit("a  |  b", "diw", "a|b");
        assert_edit("a |b c", "diw", "a | c");
        assert_edit("a |b c", "daw", "a |c");
        // `aw` on the last word takes the space before, not the indent.
        assert_edit("  |b", "daw", " | ");
    }

    #[test]
    fn brackets_in_strings_are_skipped_with_syntax_nodes() {
        let string = |v: &mut crate::test_util::TestView| v.nodes.push(0..3);
        assert_edit_with(
            Settings::default(),
            string,
            "\"(\" |+ (a)",
            "di(",
            "\"(\" + (|)",
        );
        // A `)` inside a string does not close the pair.
        let start = "f(\")\", |x)";
        assert_edit_with(
            Settings::default(),
            |v| v.nodes.push(2..5),
            start,
            "di(",
            "f(|)",
        );
        // Without the node the scan sees the `)` and finds no pair.
        assert_edit(start, "di(", start);
    }

    #[test]
    fn ci_quote_after_the_last_pair_fails() {
        let (text, actions) = run_edit("\"a\" b|c", "ci\"");
        assert_eq!(text, "\"a\" b|c");
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn escaped_quotes_are_skipped() {
        assert_edit("\"a\\\"|b\" c", "di\"", "\"|\" c");
    }

    #[test]
    fn multiline_brace_inner_is_linewise() {
        assert_edit("{\n    fo|o\n}", "di{", "{\n|}");
        assert_edit("x {\n    a|b\n    cd\n  }\ny", "di{", "x {\n  |}\ny");
        // `ci{` works like `cc`: one open row, indent kept.
        assert_edit("fn f() {\n    a|b\n    cd\n}", "ci{", "fn f() {\n    |\n}");
        assert_edit("{\n  x|\n}", "da{", "|");
        // `yi{` yanks whole rows.
        let mut g = crate::VimGlobals::default();
        let mut view = crate::test_util::TestView::new("{\n    fo|o\n}");
        let mut vim = crate::Vim::new(view.cursor);
        for k in crate::test_util::parse_keys("yi{") {
            if let crate::Outcome::Handled(a) = vim.feed(k, &view, &mut g, &Settings::default()) {
                view.apply(&mut vim, &a);
            }
        }
        let reg = g.read_register(None, crate::ClipboardMode::Never, std::path::Path::new("x"));
        assert_eq!(
            reg.map(|r| (r.text, r.kind)),
            Some(("    foo\n".to_string(), crate::RegKind::Linewise))
        );
    }

    #[test]
    fn multiline_paren_starts_at_next_row() {
        // The `)` shares its row: from the next row's start, indent kept.
        assert_edit("f(\n  |b)", "di(", "f(\n|)");
        // The opener does not end its row: the exact inner part.
        assert_edit("f(a|,\n  b)", "di(", "f(|)");
        // Nothing between `{` and `}` on the next row.
        let (text, actions) = run_edit("|{\n}", "di{");
        assert_eq!(text, "|{\n}");
        assert!(actions.is_empty());
    }

    #[test]
    fn iw_on_an_empty_line_is_empty() {
        let (text, actions) = run_edit("a\n|\nb", "diw");
        assert_eq!(text, "a\n|\nb");
        assert!(actions.is_empty(), "{actions:?}");
        let (text, actions) = run_edit("a\n|\nb", "ciw");
        assert_eq!(text, "a\n|\nb");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        assert!(actions.contains(&Action::ModeChanged(Mode::Insert)));
    }

    #[test]
    fn bracket_scan_caps_tree_queries() {
        let start = format!("{} |x", "()".repeat(5_000));
        let mut view = crate::test_util::TestView::new(&start);
        view.nodes.push(0..view.snapshot.len_bytes());
        let mut vim = crate::Vim::new(view.cursor);
        crate::test_util::feed_keys(&mut vim, &view, "di(");
        let queries = view.node_queries.get();
        assert!(queries <= 2_000, "{queries} tree queries");
        // A string found once is not asked about again.
        let start = format!("({}|)", "\"((\" ".repeat(50));
        let mut view = crate::test_util::TestView::new(&start);
        for i in 0..50 {
            let at = 1 + 5 * i;
            view.nodes.push(at..at + 4);
        }
        view.nodes.push(0..view.snapshot.len_bytes());
        let mut vim = crate::Vim::new(view.cursor);
        crate::test_util::feed_keys(&mut vim, &view, "di(");
        assert!(view.node_queries.get() <= 52, "{}", view.node_queries.get());
    }

    #[test]
    fn empty_pair_change_inserts_inside() {
        let (text, actions) = run_edit("f(|)", "ci(");
        assert_eq!(text, "f(|)");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        assert!(actions.contains(&Action::ModeChanged(Mode::Insert)));
        // `d` does nothing.
        assert_edit("f(|)", "di(", "f(|)");
    }

    #[test]
    fn tags_nest_with_count_and_use_nodes() {
        assert_edit("<a><b>x|y</b></a>", "d2it", "<a>|</a>");
        // A syntax node that is a whole element wins over the scan.
        let start = "<a> <b>|x</b> </a>";
        assert_edit_with(
            Settings::default(),
            |v| v.nodes = vec![4..12, 0..17],
            start,
            "dat",
            "<a> | </a>",
        );
    }

    #[test]
    fn yank_object_moves_cursor_to_start() {
        assert_edit("one tw|o", "yiw", "one |two");
    }

    #[test]
    fn readonly_view_gets_no_edit() {
        let (text, actions) =
            run_edit_with(Settings::default(), |v| v.readonly = true, "a(|b)", "ci(");
        assert_eq!(text, "a(|b)");
        assert!(actions.is_empty());
    }

    fn sel_after(start: &str, keys: &str) -> (Option<VimSelection>, Mode) {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, start, keys);
        (vim.selection(&view.snapshot, 4), vim.mode())
    }

    fn p(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    #[test]
    fn viw_extends_the_selection() {
        let char_sel = |a, b| {
            Some(VimSelection::Char {
                start: a,
                end_inclusive: b,
            })
        };
        assert_eq!(
            sel_after("one t|wo three", "viw"),
            (char_sel(p(0, 4), p(0, 6)), Mode::Visual)
        );
        // Again: the white space after it joins.
        assert_eq!(
            sel_after("one t|wo three", "viwiw"),
            (char_sel(p(0, 4), p(0, 7)), Mode::Visual)
        );
        // From a wider selection, the word under the head joins it.
        assert_eq!(
            sel_after("o|ne two three", "vwiw"),
            (char_sel(p(0, 1), p(0, 6)), Mode::Visual)
        );
        assert_eq!(
            sel_after("f((a|b))", "vi(a("),
            (char_sel(p(0, 2), p(0, 5)), Mode::Visual)
        );
        // `i(` again on its own inner part grows to the next pair out.
        assert_eq!(
            sel_after("f((a|b))", "vi(i("),
            (char_sel(p(0, 2), p(0, 5)), Mode::Visual)
        );
    }

    #[test]
    fn vi_brace_over_rows_is_charwise() {
        assert_eq!(
            sel_after("{\n  a|b\n  c\n}", "vi{"),
            (
                Some(VimSelection::Char {
                    start: p(1, 0),
                    end_inclusive: p(2, 3)
                }),
                Mode::Visual
            )
        );
    }

    #[test]
    fn raw_strings_are_strings() {
        let start = "f(r#\")\"#, |x)";
        assert_edit_with(
            Settings::default(),
            |v| v.nodes.push(2..8),
            start,
            "di(",
            "f(|)",
        );
    }

    #[test]
    fn vip_switches_to_visual_line() {
        assert_eq!(
            sel_after("a\n|b\nc\n\nd", "vip"),
            (
                Some(VimSelection::Line {
                    start_row: 0,
                    end_row: 2
                }),
                Mode::VisualLine
            )
        );
        assert_eq!(
            sel_after("a\n|b\nc\n\nd", "vap").0,
            Some(VimSelection::Line {
                start_row: 0,
                end_row: 3
            })
        );
    }

    #[test]
    fn sentences_and_paragraph_edges() {
        assert_edit("One. T|wo. Three.", "dis", "One. | Three.");
        assert_edit("One. T|wo. Three.", "das", "One. |Three.");
        // The last sentence has no white space after: `as` takes it before.
        assert_edit("One. Tw|o.", "das", "One|.");
        // `ap` on the last paragraph takes the blank lines before it.
        assert_edit("a\n\nb|c", "dap", "|a");
        assert_edit("|a\nb\n\nc\n\nd", "d2ap", "|d");
        assert_edit("a\n|\nb", "dip", "a\n|b");
    }
}
