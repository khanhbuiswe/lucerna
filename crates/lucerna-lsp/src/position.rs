//! Conversion between Lucerna points (UTF-8 byte columns) and LSP positions.

use std::ops::Range;
use std::path::{Path, PathBuf};

use lucerna_text::{BufferSnapshot, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

impl PositionEncoding {
    /// `None` or an unknown kind (including utf-32) falls back to UTF-16,
    /// the LSP default.
    pub fn from_lsp(kind: Option<&lsp_types::PositionEncodingKind>) -> Self {
        match kind {
            Some(k) if *k == lsp_types::PositionEncodingKind::UTF8 => Self::Utf8,
            _ => Self::Utf16,
        }
    }

    fn units(self, c: char) -> u32 {
        match self {
            Self::Utf8 => c.len_utf8() as u32,
            Self::Utf16 => c.len_utf16() as u32,
        }
    }
}

/// LSP `character` to a byte column. Clamped to the line length; a position
/// inside a character maps to the start of that character.
pub fn column_from_lsp(line: &str, character: u32, enc: PositionEncoding) -> u32 {
    let mut units = 0u32;
    for (byte, c) in line.char_indices() {
        let next = units + enc.units(c);
        if next > character {
            return byte as u32;
        }
        units = next;
    }
    line.len() as u32
}

/// Byte column to LSP `character`. A column inside a character counts that
/// character's start; past the end clamps to the line length.
pub fn column_to_lsp(line: &str, byte_column: u32, enc: PositionEncoding) -> u32 {
    let mut units = 0u32;
    for (byte, c) in line.char_indices() {
        if byte as u32 >= byte_column {
            break;
        }
        // A column strictly inside this char maps to its start.
        if (byte + c.len_utf8()) as u32 > byte_column {
            break;
        }
        units += enc.units(c);
    }
    units
}

pub fn to_lsp(snapshot: &BufferSnapshot, p: Point, enc: PositionEncoding) -> lsp_types::Position {
    let line = snapshot.line(p.row);
    lsp_types::Position {
        line: p.row,
        character: column_to_lsp(&line, p.column, enc),
    }
}

/// Rows past the end clamp to the end of the last row.
pub fn from_lsp(snapshot: &BufferSnapshot, p: lsp_types::Position, enc: PositionEncoding) -> Point {
    let last = snapshot.line_count().saturating_sub(1);
    if p.line > last {
        return Point {
            row: last,
            column: snapshot.line(last).len() as u32,
        };
    }
    let line = snapshot.line(p.line);
    Point {
        row: p.line,
        column: column_from_lsp(&line, p.character, enc),
    }
}

pub fn range_from_lsp(
    snapshot: &BufferSnapshot,
    r: lsp_types::Range,
    enc: PositionEncoding,
) -> Range<Point> {
    let start = from_lsp(snapshot, r.start, enc);
    let end = from_lsp(snapshot, r.end, enc);
    // Some servers send reversed ranges; never hand out start > end.
    if (end.row, end.column) < (start.row, start.column) {
        end..start
    } else {
        start..end
    }
}

pub fn path_to_uri(path: &Path) -> lsp_types::Uri {
    // `from_file_path` fails only for relative paths; paths are absolute
    // everywhere in Lucerna, and a bare `file:///` is a safe fallback.
    let url = url::Url::from_file_path(path)
        .unwrap_or_else(|()| url::Url::parse("file:///").expect("static url"));
    url.as_str()
        .parse()
        .expect("a url::Url is a valid RFC 3986 URI")
}

/// `file://` URIs only; percent-decoded.
pub fn uri_to_path(uri: &lsp_types::Uri) -> Option<PathBuf> {
    let url = url::Url::parse(uri.as_str()).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use PositionEncoding::{Utf8, Utf16};
    use lucerna_text::Buffer;

    fn snap(text: &str) -> BufferSnapshot {
        Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot()
    }

    #[test]
    fn utf16_counts_surrogate_pairs() {
        let line = "a😀b";
        assert_eq!(column_to_lsp(line, 5, Utf16), 3);
        assert_eq!(column_from_lsp(line, 3, Utf16), 5);
    }

    #[test]
    fn cjk_round_trip() {
        let line = "日本語x";
        for enc in [Utf8, Utf16] {
            for (byte, _) in line.char_indices().chain([(line.len(), 'x')]) {
                let ch = column_to_lsp(line, byte as u32, enc);
                assert_eq!(column_from_lsp(line, ch, enc), byte as u32, "{enc:?}");
            }
        }
    }

    #[test]
    fn utf8_is_identity() {
        assert_eq!(column_to_lsp("héllo", 3, Utf8), 3);
        assert_eq!(column_from_lsp("héllo", 3, Utf8), 3);
    }

    #[test]
    fn out_of_range_clamps() {
        let s = snap("ab\ncde");
        let p = from_lsp(&s, lsp_types::Position::new(99, 0), Utf16);
        assert_eq!(p, Point { row: 1, column: 3 });
        let p = from_lsp(&s, lsp_types::Position::new(0, 999), Utf16);
        assert_eq!(p, Point { row: 0, column: 2 });
        // Inside a surrogate pair: start of that char.
        assert_eq!(column_from_lsp("a😀b", 2, Utf16), 1);
        assert_eq!(column_to_lsp("a😀b", 3, Utf16), 1);
    }

    #[test]
    fn crlf_line_end_excluded() {
        let s = snap("ab\r\ncd");
        let p = from_lsp(&s, lsp_types::Position::new(0, 5), Utf16);
        assert_eq!(p, Point { row: 0, column: 2 });
    }

    #[test]
    fn range_and_snapshot_round_trip() {
        let s = snap("a😀b\nxy");
        let pt = Point { row: 0, column: 5 };
        let lsp = to_lsp(&s, pt, Utf16);
        assert_eq!(lsp, lsp_types::Position::new(0, 3));
        let r = lsp_types::Range::new(lsp, lsp_types::Position::new(1, 1));
        assert_eq!(
            range_from_lsp(&s, r, Utf16),
            pt..Point { row: 1, column: 1 }
        );
    }

    #[test]
    fn reversed_range_is_swapped() {
        let s = snap("abc\ndef");
        let r = lsp_types::Range::new(
            lsp_types::Position::new(1, 2),
            lsp_types::Position::new(0, 1),
        );
        assert_eq!(
            range_from_lsp(&s, r, Utf16),
            Point { row: 0, column: 1 }..Point { row: 1, column: 2 }
        );
    }

    #[test]
    fn encoding_from_lsp() {
        assert_eq!(PositionEncoding::from_lsp(None), Utf16);
        assert_eq!(
            PositionEncoding::from_lsp(Some(&lsp_types::PositionEncodingKind::UTF8)),
            Utf8
        );
        assert_eq!(
            PositionEncoding::from_lsp(Some(&lsp_types::PositionEncodingKind::UTF32)),
            Utf16
        );
    }

    #[cfg(unix)]
    #[test]
    fn uri_round_trip() {
        let path = Path::new("/tmp/a b/ü.rs");
        let uri = path_to_uri(path);
        assert_eq!(uri.as_str(), "file:///tmp/a%20b/%C3%BC.rs");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
        let other: lsp_types::Uri = "untitled:x".parse().unwrap();
        assert_eq!(uri_to_path(&other), None);
    }
}
