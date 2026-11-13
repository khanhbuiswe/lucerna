//! Text objects (spec §5): the ranges `iw`, `a(`, `i"`, `it`, `ap`, `is`
//! and the rest cover, after an operator or in Visual mode.

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

use crate::ViewInfo;
use crate::classify::{CharClass, class, line};
use crate::operator::{MotionKind, OpRange, row_start};
use crate::visual::{line_end, next_row_start};

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
/// Paragraphs are linewise (from the start of the first row to the end of
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
        Object::Paren => bracket(s, cursor, (b'(', b')'), around, count, view).map(chars),
        Object::Brace => bracket(s, cursor, (b'{', b'}'), around, count, view).map(chars),
        Object::Bracket => bracket(s, cursor, (b'[', b']'), around, count, view).map(chars),
        Object::Angle => bracket(s, cursor, (b'<', b'>'), around, count, view).map(chars),
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

/// A position walked char by char across rows, the way Vim walks words: a
/// row's end (its line break) is a position of its own, of class
/// [`CharClass::Space`].
struct Walk<'a> {
    s: &'a BufferSnapshot,
    big: bool,
    last_row: u32,
    row: u32,
    col: usize,
    text: String,
}

/// What a step did: stayed in the row (`Same`), landed on the row's end
/// (`Eol`), went to the next or previous row (`Row`), or could not move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Same,
    Eol,
    Row,
    Stuck,
}

impl<'a> Walk<'a> {
    fn new(s: &'a BufferSnapshot, p: Point, big: bool) -> Walk<'a> {
        let mut w = Walk {
            s,
            big,
            last_row: s.line_count().saturating_sub(1),
            row: u32::MAX,
            col: 0,
            text: String::new(),
        };
        w.goto(p);
        w
    }

    fn goto(&mut self, p: Point) {
        if p.row != self.row {
            self.row = p.row;
            self.text = line(self.s, p.row);
        }
        let mut col = (p.column as usize).min(self.text.len());
        while !self.text.is_char_boundary(col) {
            col -= 1;
        }
        self.col = col;
    }

    fn pos(&self) -> Point {
        Point {
            row: self.row,
            column: self.col as u32,
        }
    }

    fn cls(&self) -> CharClass {
        self.text[self.col..]
            .chars()
            .next()
            .map_or(CharClass::Space, |c| class(c, self.big))
    }

    fn on_empty_line(&self) -> bool {
        self.text.is_empty()
    }

    fn inc(&mut self) -> Step {
        if let Some(c) = self.text[self.col..].chars().next() {
            self.col += c.len_utf8();
            return if self.col < self.text.len() {
                Step::Same
            } else {
                Step::Eol
            };
        }
        if self.row < self.last_row {
            self.goto(Point {
                row: self.row + 1,
                column: 0,
            });
            return Step::Row;
        }
        Step::Stuck
    }

    fn dec(&mut self) -> Step {
        if let Some(c) = self.text[..self.col].chars().next_back() {
            self.col -= c.len_utf8();
            return Step::Same;
        }
        if self.row > 0 {
            self.goto(Point {
                row: self.row - 1,
                column: u32::MAX,
            });
            return Step::Row;
        }
        Step::Stuck
    }

    /// Like [`Walk::inc`], but steps over a row's end onto the next row.
    fn incl(&mut self) -> Step {
        let r = self.inc();
        if r == Step::Eol {
            return self.inc();
        }
        r
    }

    /// Like [`Walk::dec`], but steps over the previous row's end onto its
    /// last char.
    fn decl(&mut self) -> Step {
        let r = self.dec();
        if r == Step::Row && self.col > 0 {
            return self.dec();
        }
        r
    }

    /// One char left within the row; false at column 0.
    fn oneleft(&mut self) -> bool {
        self.col > 0 && self.dec() == Step::Same
    }

    /// Back to the first char of the run of this class, within the row.
    fn back_in_line(&mut self) {
        let start = self.cls();
        while self.col > 0 {
            self.dec();
            if self.cls() != start {
                self.inc();
                break;
            }
        }
    }

    /// To the start of the next word, stopping at a row's end (Vim's
    /// `fwd_word` with `eol`) and on an empty row. False when it cannot
    /// leave the end of the buffer.
    fn fwd_word(&mut self) -> bool {
        let start = self.cls();
        let last_row = self.row == self.last_row;
        match self.inc() {
            Step::Stuck => return false,
            Step::Eol | Step::Row if last_row => return false,
            Step::Eol | Step::Row => return true,
            Step::Same => {}
        }
        if start != CharClass::Space {
            while self.cls() == start {
                if self.inc() != Step::Same {
                    return true;
                }
            }
        }
        while self.cls() == CharClass::Space {
            if self.col == 0 && self.on_empty_line() {
                break;
            }
            if self.inc() != Step::Same {
                return true;
            }
        }
        true
    }

    /// To the end of the word (Vim's `end_word` with `stop` and `empty`):
    /// a cursor already on a word's end stays; from white space, to the end
    /// of the next word, or onto an empty row.
    fn end_word(&mut self) -> bool {
        let start = self.cls();
        if self.inc() == Step::Stuck {
            return false;
        }
        if self.cls() == start && start != CharClass::Space {
            if !self.skip(start) {
                return false;
            }
        } else if start == CharClass::Space {
            while self.cls() == CharClass::Space {
                if self.col == 0 && self.on_empty_line() {
                    return true;
                }
                if self.inc() == Step::Stuck {
                    return false;
                }
            }
            if !self.skip(self.cls()) {
                return false;
            }
        }
        self.dec();
        true
    }

    /// Forward past the run of class `c`; false at the end of the buffer.
    fn skip(&mut self, c: CharClass) -> bool {
        while self.cls() == c {
            if self.inc() == Step::Stuck {
                return false;
            }
        }
        true
    }

    /// The buffer offset of the position, a row's end being its line break.
    fn offset(&self) -> usize {
        if self.col >= self.text.len() {
            line_end(self.s, self.row)
        } else {
            self.s.point_to_offset(self.pos())
        }
    }

    /// The offset after the char at the position (after the line break at
    /// a row's end).
    fn offset_after(&self) -> usize {
        match self.text[self.col..].chars().next() {
            Some(c) => self.s.point_to_offset(self.pos()) + c.len_utf8(),
            None => next_row_start(self.s, self.row),
        }
    }
}

/// `iw` `aw` `iW` `aW`, as Vim's `current_word` defines them: `iw` is the
/// run of word, punctuation or white space under the cursor; `aw` adds the
/// white space after (or the one before when there is none after, unless
/// it is the indent); on white space `aw` is the white space and the next
/// word. A count takes more runs, across rows.
fn word(
    s: &BufferSnapshot,
    cursor: Point,
    big: bool,
    around: bool,
    count: u32,
) -> Option<Range<usize>> {
    let mut w = Walk::new(s, cursor, big);
    w.back_in_line();
    let mut start = w.pos();
    let mut include_white = false;
    if (w.cls() == CharClass::Space) == around {
        if !w.end_word() {
            return None;
        }
    } else {
        w.fwd_word();
        if w.col == 0 {
            w.decl();
        } else {
            w.oneleft();
        }
        include_white = around;
    }
    let mut inclusive = true;
    let mut left = count - 1;
    while left > 0 {
        inclusive = true;
        if w.incl() == Step::Stuck {
            return None;
        }
        if around != (w.cls() == CharClass::Space) {
            if !w.fwd_word() && left > 1 {
                return None;
            }
            // Ending just past a line break: the next row's first char is
            // not taken.
            if !w.oneleft() {
                inclusive = false;
            }
        } else if !w.end_word() {
            return None;
        }
        left -= 1;
    }
    if include_white && (w.cls() != CharClass::Space || (w.col == 0 && !inclusive)) {
        // No white space after: take the white space before, unless it is
        // the indent.
        let end = w.pos();
        w.goto(start);
        if w.oneleft() {
            w.back_in_line();
            if w.cls() == CharClass::Space && w.col > 0 {
                start = w.pos();
            }
        }
        w.goto(end);
    }
    let from = s.point_to_offset(start);
    let to = if inclusive {
        w.offset_after()
    } else {
        w.offset()
    };
    Some(from..to.max(from))
}

// ---------------------------------------------------------------------------
// Brackets

/// Whether the node starting at `start` reads as a string or a comment. A
/// node here is only a byte range, so its text decides:
/// - a string starts with a quote, after at most two ASCII letters (`r"`,
///   `b'`, `f"`, `rb"`), and ends with that quote, perhaps followed by `#`s
///   (a Rust raw string). Requiring the closing quote keeps out nodes that
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
}

impl Brackets<'_> {
    /// The string or comment node the bracket at window index `i` lies
    /// strictly inside, unless that node also holds the cursor.
    fn skipped(&self, i: usize) -> Option<Range<usize>> {
        let at = self.lo + i;
        self.view.enclosing_nodes(at..at + 1).into_iter().find(|n| {
            n.start < at
                && at < n.end
                && !(n.start <= self.cursor && self.cursor < n.end)
                && is_string_or_comment(self.s, n)
        })
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
/// The inner part of a pair that spans rows drops the white space (line
/// breaks included) at its start and end, so the row of the closing
/// bracket is not taken.
fn bracket(
    s: &BufferSnapshot,
    cursor: Point,
    pair: (u8, u8),
    around: bool,
    count: u32,
    view: &dyn ViewInfo,
) -> Option<Range<usize>> {
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
    };
    let cur = at - lo;
    let (open, close) = brackets
        .enclosing(cur, count)
        .or_else(|| brackets.next_pair(cur))?;
    if around {
        return Some(lo + open..lo + close + 1);
    }
    let inner = &brackets.bytes[open + 1..close];
    if !inner.contains(&b'\n') {
        return Some(lo + open + 1..lo + close);
    }
    let lead = inner.iter().take_while(|b| b.is_ascii_whitespace()).count();
    let trail = inner
        .iter()
        .rev()
        .take_while(|b| b.is_ascii_whitespace())
        .count();
    let from = lo + open + 1 + lead;
    let to = (lo + close - trail).max(from);
    Some(from..to)
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

/// The element a syntax node covers, if its text is `<name …>…</name>`.
fn element_node(s: &BufferSnapshot, node: &Range<usize>) -> Option<Element> {
    // A tree a parse behind the text may hold ranges that no longer fit.
    let fits = |at: usize| at <= s.len_bytes() && s.floor_char_boundary(at) == at;
    if node.start >= node.end || !fits(node.start) || !fits(node.end) {
        return None;
    }
    let bytes = s.text_for_range(node.clone()).into_bytes();
    let name = tag_name_len(bytes.get(1..)?);
    if bytes[0] != b'<' || name == 0 {
        return None;
    }
    let open_end = tag_end(&bytes, 1 + name)?;
    let name = &bytes[1..1 + name];
    let body = bytes.strip_suffix(b">")?;
    let body = body.trim_ascii_end();
    let close_start = body.strip_suffix(name)?.strip_suffix(b"</")?.len();
    (close_start >= open_end).then(|| {
        (
            node.start..node.start + open_end,
            node.start + close_start..node.end,
        )
    })
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
    fn multiline_brace_inner_drops_surrounding_whitespace() {
        assert_edit("fn f() {\n    a|b\n    cd\n}", "ci{", "fn f() {\n    |\n}");
        assert_edit("{\n  x|\n}", "da{", "|");
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
