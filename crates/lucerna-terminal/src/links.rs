//! Link detection on the visible screen: OSC 8, URLs, then file paths with an
//! optional line/column suffix. Pure and gpui-free; the view calls `link_at`
//! on hover with a 1 ms budget.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::snapshot::{Cell, CellFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    Url(String),
    Path {
        path: String,
        line: Option<u32>,
        col: Option<u32>,
    },
}

/// A link and its span as `(row, col)` cell positions, inclusive, in
/// snapshot coordinates. May cross rows when the line wrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    pub start: (u16, u16),
    pub end: (u16, u16),
}

/// One character of the logical line and the screen cells it occupies.
struct Ent {
    row: u16,
    col: u16,
    /// Last cell column (`col + 1` for a wide character).
    end: u16,
    link: Option<Arc<str>>,
}

struct Logical {
    text: String,
    ents: Vec<Ent>,
    /// Byte offset in `text` to index in `ents`; `text.len() + 1` entries.
    char_of_byte: Vec<usize>,
    /// Index in `ents` to byte offset in `text`.
    byte_of_char: Vec<usize>,
}

/// Joins the wrapped rows around `row` into one logical line.
fn logical_line(rows: &[Vec<Cell>], wrapped: &[bool], row: usize) -> Logical {
    let wrap = |i: usize| wrapped.get(i).copied().unwrap_or(false);
    let mut first = row;
    while first > 0 && wrap(first - 1) {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < rows.len() && wrap(last) {
        last += 1;
    }

    let mut text = String::new();
    let mut ents = Vec::new();
    let mut char_of_byte = Vec::new();
    let mut byte_of_char = Vec::new();
    let mut push = |ch: char, ent: Ent, text: &mut String| {
        byte_of_char.push(text.len());
        for _ in 0..ch.len_utf8() {
            char_of_byte.push(ents.len());
        }
        text.push(ch);
        ents.push(ent);
    };
    for (r, cells) in rows.iter().enumerate().take(last + 1).skip(first) {
        for (c, cell) in cells.iter().enumerate() {
            if cell.flags.contains(CellFlags::WIDE_SPACER) {
                continue;
            }
            let end = if cell.flags.contains(CellFlags::WIDE) {
                c + 1
            } else {
                c
            };
            let ent = |link: &Option<Arc<str>>| Ent {
                row: r as u16,
                col: c as u16,
                end: end as u16,
                link: link.clone(),
            };
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            push(ch, ent(&cell.hyperlink), &mut text);
            for &z in cell.zerowidth.iter().flat_map(|z| z.iter()) {
                push(z, ent(&cell.hyperlink), &mut text);
            }
        }
    }
    char_of_byte.push(ents.len());
    Logical {
        text,
        ents,
        char_of_byte,
        byte_of_char,
    }
}

fn span(line: &Logical, first: usize, last: usize, kind: LinkKind) -> Link {
    let (a, b) = (&line.ents[first], &line.ents[last]);
    Link {
        kind,
        start: (a.row, a.col),
        end: (b.row, b.end),
    }
}

/// The link under cell `(row, col)`, or `None`. Gives up (returns `None`) once
/// `budget` is spent, checked between passes.
pub fn link_at(
    rows: &[Vec<Cell>],
    wrapped: &[bool],
    row: u16,
    col: u16,
    budget: Duration,
) -> Option<Link> {
    if budget.is_zero() {
        return None;
    }
    let started = Instant::now();
    let row_cells = rows.get(row as usize)?;
    if (col as usize) >= row_cells.len() {
        return None;
    }
    let line = logical_line(rows, wrapped, row as usize);
    let target = line
        .ents
        .iter()
        .position(|e| e.row == row && e.col <= col && col <= e.end)?;
    let ch = line.text[line.byte_of_char[target]..].chars().next()?;
    if ch.is_whitespace() {
        return None;
    }
    // The byte range of a regex match as the first and last entry it covers.
    let covers = |start: usize, end: usize| -> Option<(usize, usize)> {
        let (a, b) = (line.char_of_byte[start], line.char_of_byte[end - 1]);
        (a <= target && target <= b).then_some((a, b))
    };

    // 1. OSC 8.
    if let Some(uri) = &line.ents[target].link {
        let same = |i: usize| line.ents[i].link.as_deref() == Some(&**uri);
        let mut lo = target;
        while lo > 0 && same(lo - 1) {
            lo -= 1;
        }
        let mut hi = target;
        while hi + 1 < line.ents.len() && same(hi + 1) {
            hi += 1;
        }
        return Some(span(&line, lo, hi, LinkKind::Url(uri.to_string())));
    }
    if started.elapsed() > budget {
        return None;
    }

    // 2. URLs.
    for m in url_re().find_iter(&line.text) {
        let url = trim_url(m.as_str());
        if url.is_empty() {
            continue;
        }
        if let Some((a, b)) = covers(m.start(), m.start() + url.len()) {
            return Some(span(&line, a, b, LinkKind::Url(url.to_string())));
        }
    }
    if started.elapsed() > budget {
        return None;
    }

    // 3. Python traceback: only the quoted path is the link.
    for caps in python_re().captures_iter(&line.text) {
        let p = caps.name("path")?;
        if let Some((a, b)) = covers(p.start(), p.end()) {
            let line_no = caps.name("line").and_then(|m| m.as_str().parse().ok());
            let kind = LinkKind::Path {
                path: p.as_str().to_string(),
                line: line_no,
                col: None,
            };
            return Some(span(&line, a, b, kind));
        }
    }
    if started.elapsed() > budget {
        return None;
    }

    // 4. Paths, with or without a position suffix.
    for caps in path_re().captures_iter(&line.text) {
        let whole = caps.get(0)?;
        let p = caps.name("path")?;
        let number = |names: [&str; 3]| {
            names
                .iter()
                .find_map(|n| caps.name(n))
                .and_then(|m| m.as_str().parse::<u32>().ok())
        };
        let (line_no, col_no) = (number(["l1", "l2", "l3"]), number(["c1", "c2", "c2"]));
        let has_suffix = whole.end() > p.end();
        let mut path = p.as_str();
        let mut end = whole.end();
        if !has_suffix {
            path = path.trim_end_matches('.');
            end = p.start() + path.len();
        }
        if end == p.start() || !looks_like_path(path) {
            continue;
        }
        if let Some((a, b)) = covers(whole.start(), end) {
            let kind = LinkKind::Path {
                path: path.to_string(),
                line: line_no,
                col: col_no,
            };
            return Some(span(&line, a, b, kind));
        }
    }
    None
}

/// A path needs a directory separator or a file extension, and a word
/// character: that rules out `...`, `1.2.3` and plain words.
fn looks_like_path(path: &str) -> bool {
    if !path.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }
    path.contains('/') || ext_re().is_match(path)
}

/// Drops trailing punctuation and closers that do not match an opener.
fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let Some(last) = url.chars().next_back() else {
            return url;
        };
        let count = |c: char| url.matches(c).count();
        let trim = match last {
            '.' | ',' | ';' | ':' | '!' | '?' => true,
            ')' => count(')') > count('('),
            ']' => count(']') > count('['),
            '\'' => count('\'') % 2 == 1,
            _ => false,
        };
        if !trim {
            return url;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?:https?://|file://|mailto:)[^\s<>"`]+"#).expect("url regex"))
}

fn python_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"File "(?P<path>[^"]+)", line (?P<line>\d+)"#).expect("python regex")
    })
}

/// The suffix forms follow VS Code's terminal link patterns (MIT): `:L`,
/// `:L:C`, `(L)`, `(L,C)` and `, line L`.
fn path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            (?P<path>(?:~/|\.{1,2}/|/)?[\w.@+%~-]+(?:/[\w.@+%~-]+)*/?)
            (?:
                :(?P<l1>\d+)(?::(?P<c1>\d+))?
              | \((?P<l2>\d+)(?:,\s*(?P<c2>\d+))?\)
              | ,\s*line\s+(?P<l3>\d+)
            )?",
        )
        .expect("path regex")
    })
}

fn ext_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.[A-Za-z]\w*$").expect("ext regex"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{CellFlags, NamedColor, TermColor};
    const BUDGET: Duration = Duration::from_millis(50);

    fn cell(ch: char) -> Cell {
        Cell {
            ch,
            zerowidth: None,
            fg: TermColor::Named(NamedColor::Foreground),
            bg: TermColor::Named(NamedColor::Background),
            flags: CellFlags::empty(),
            hyperlink: None,
        }
    }

    fn row(s: &str) -> Vec<Cell> {
        s.chars().map(cell).collect()
    }

    fn at(s: &str, col: u16) -> Option<Link> {
        link_at(&[row(s)], &[false], 0, col, BUDGET)
    }

    fn path(p: &str, line: Option<u32>, col: Option<u32>) -> LinkKind {
        LinkKind::Path {
            path: p.into(),
            line,
            col,
        }
    }

    #[test]
    fn osc8_wins() {
        let uri: Arc<str> = Arc::from("https://example.com/x");
        let mut r = row("go here now");
        for c in &mut r[3..7] {
            c.hyperlink = Some(uri.clone());
        }
        let l = link_at(&[r], &[false], 0, 5, BUDGET).unwrap();
        assert_eq!(l.kind, LinkKind::Url("https://example.com/x".into()));
        assert_eq!((l.start, l.end), ((0, 3), (0, 6)));
    }

    #[test]
    fn osc8_beats_path_text() {
        let uri: Arc<str> = Arc::from("https://e.com");
        let mut r = row("src/main.rs");
        for c in &mut r {
            c.hyperlink = Some(uri.clone());
        }
        let l = link_at(&[r], &[false], 0, 2, BUDGET).unwrap();
        assert_eq!(l.kind, LinkKind::Url("https://e.com".into()));
    }

    #[test]
    fn url_detected() {
        let s = "see https://example.com/a?b=1.";
        let l = at(s, 10).unwrap();
        assert_eq!(l.kind, LinkKind::Url("https://example.com/a?b=1".into()));
        assert_eq!((l.start, l.end), ((0, 4), (0, 28)));
        assert!(at(s, 1).is_none());
    }

    #[test]
    fn url_trims_unbalanced_closers() {
        let l = at("(see https://e.com/a)", 8).unwrap();
        assert_eq!(l.kind, LinkKind::Url("https://e.com/a".into()));
        let l = at("https://en.wikipedia.org/wiki/Foo_(bar)", 8).unwrap();
        assert_eq!(
            l.kind,
            LinkKind::Url("https://en.wikipedia.org/wiki/Foo_(bar)".into())
        );
        let l = at("\"https://e.com/a\", next", 5).unwrap();
        assert_eq!(l.kind, LinkKind::Url("https://e.com/a".into()));
    }

    #[test]
    fn mailto_and_file_schemes() {
        let l = at("mail mailto:a@b.com now", 8).unwrap();
        assert_eq!(l.kind, LinkKind::Url("mailto:a@b.com".into()));
        let l = at("file:///tmp/x.txt", 3).unwrap();
        assert_eq!(l.kind, LinkKind::Url("file:///tmp/x.txt".into()));
    }

    #[test]
    fn path_with_line_col() {
        let l = at("src/main.rs:12:5", 4).unwrap();
        assert_eq!(l.kind, path("src/main.rs", Some(12), Some(5)));
        assert_eq!((l.start, l.end), ((0, 0), (0, 15)));
    }

    #[test]
    fn path_with_line_only_and_trailing_colon() {
        let l = at("error: src/lib.rs:7: boom", 10).unwrap();
        assert_eq!(l.kind, path("src/lib.rs", Some(7), None));
    }

    #[test]
    fn path_paren_form() {
        let l = at("foo.ts(3,7)", 2).unwrap();
        assert_eq!(l.kind, path("foo.ts", Some(3), Some(7)));
        let l = at("foo.ts(3)", 2).unwrap();
        assert_eq!(l.kind, path("foo.ts", Some(3), None));
    }

    #[test]
    fn path_comma_line_form() {
        let l = at("foo.py, line 9", 2).unwrap();
        assert_eq!(l.kind, path("foo.py", Some(9), None));
    }

    #[test]
    fn path_without_location() {
        let l = at("open ./a/b.txt please", 8).unwrap();
        assert_eq!(l.kind, path("./a/b.txt", None, None));
        let l = at("see src/main.rs.", 6).unwrap();
        assert_eq!(l.kind, path("src/main.rs", None, None));
        assert_eq!((l.start, l.end), ((0, 4), (0, 14)));
        let l = at("edit Cargo.toml", 7).unwrap();
        assert_eq!(l.kind, path("Cargo.toml", None, None));
    }

    #[test]
    fn python_traceback() {
        let s = "  File \"app/x.py\", line 42, in main";
        let l = at(s, 10).unwrap();
        assert_eq!(l.kind, path("app/x.py", Some(42), None));
        assert_eq!((l.start, l.end), ((0, 8), (0, 15)));
    }

    #[test]
    fn git_prefix_kept_raw() {
        let l = at("a/src/x.rs", 3).unwrap();
        assert_eq!(l.kind, path("a/src/x.rs", None, None));
    }

    #[test]
    fn link_wrapped_across_rows() {
        let rows = [row("see src/ma"), row("in.rs:3 ok ")];
        let wrapped = [true, false];
        for (r, c) in [(0, 6), (1, 2)] {
            let l = link_at(&rows, &wrapped, r, c, BUDGET).unwrap();
            assert_eq!(l.kind, path("src/main.rs", Some(3), None));
            assert_eq!((l.start, l.end), ((0, 4), (1, 6)));
        }
        // Without the wrap the halves are separate.
        assert!(link_at(&rows, &[false, false], 1, 1, BUDGET).is_some());
        let l = link_at(&rows, &[false, false], 0, 6, BUDGET).unwrap();
        assert_eq!(l.kind, path("src/ma", None, None));
    }

    #[test]
    fn wide_chars_use_cell_columns() {
        let mut r = vec![cell('世')];
        r[0].flags = CellFlags::WIDE;
        let mut sp = cell(' ');
        sp.flags = CellFlags::WIDE_SPACER;
        r.push(sp);
        r.extend(row(" src/a.rs"));
        let l = link_at(&[r], &[false], 0, 6, BUDGET).unwrap();
        assert_eq!(l.kind, path("src/a.rs", None, None));
        assert_eq!((l.start, l.end), ((0, 3), (0, 10)));
    }

    #[test]
    fn no_link_in_plain_word() {
        assert!(at("hello world", 2).is_none());
        assert!(at("version 1.2.3 ok", 10).is_none());
        assert!(at("a ... b", 3).is_none());
    }

    #[test]
    fn click_on_space_is_none() {
        assert!(at("src/a.rs  src/b.rs", 9).is_none());
    }

    #[test]
    fn budget_zero_returns_none() {
        let r = row("src/main.rs:12:5");
        assert!(link_at(&[r], &[false], 0, 3, Duration::ZERO).is_none());
    }

    #[test]
    fn out_of_range_is_none() {
        assert!(at("src/main.rs", 40).is_none());
        assert!(link_at(&[row("x")], &[false], 5, 0, BUDGET).is_none());
        assert!(link_at(&[], &[], 0, 0, BUDGET).is_none());
    }
}
