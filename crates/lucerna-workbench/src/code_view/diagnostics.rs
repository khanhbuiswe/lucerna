//! What a [`super::CodeView`] paints for its diagnostics (spec §7.1),
//! built once per diagnostics change or new text, never per frame:
//! underline marks, the gutter icon and inline message of each row, and
//! the scrollbar marks.

use std::collections::BTreeMap;
use std::ops::Range;

use lucerna_lsp::diagnostics::{Diagnostic, Severity};
use lucerna_text::{BufferSnapshot, Point};

/// How a [`Mark`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkStyle {
    /// A wavy underline in the severity's color.
    Wavy,
    /// The text faded (an `Unnecessary` tag below warning).
    Fade,
    /// A line through the text (a `Deprecated` tag).
    Strike,
}

/// One decoration of a diagnostic's range, clamped to the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub range: Range<Point>,
    pub severity: Severity,
    pub style: MarkStyle,
}

/// The most chars of an inline message kept: it is clipped at the
/// editor's right edge anyway, and shaped on every frame it shows.
const INLINE_MAX_CHARS: usize = 500;

/// One row's summary: its gutter icon and inline message.
#[derive(Debug)]
struct RowInfo {
    row: u32,
    /// The highest severity of Error, Warning and Information.
    gutter: Option<Severity>,
    /// The most severe non-hint diagnostic's first message line.
    inline: Option<(Severity, String)>,
}

/// The diagnostics of one view, as painted. Rows are those of each
/// diagnostic's start; every range is clamped to the text it was built
/// for (stale points after a reload never reach past it).
#[derive(Debug, Default)]
pub struct DiagnosticRows {
    /// Sorted by range start.
    marks: Vec<Mark>,
    /// The most rows any mark spans beyond its first.
    max_span: u32,
    /// Sorted by row; rows with only hints have no entry.
    rows: Vec<RowInfo>,
    scrollbar: Vec<(u32, Severity)>,
}

impl DiagnosticRows {
    pub fn build(diags: &[Diagnostic], snapshot: &BufferSnapshot) -> DiagnosticRows {
        let mut marks = Vec::new();
        // Keyed by row: one lookup per diagnostic, O(n log n) overall.
        let mut rows: BTreeMap<u32, RowInfo> = BTreeMap::new();
        for d in diags {
            let range = clamp_range(snapshot, d.range.clone());
            let row = range.start.row;
            if let Some(range) = widen_empty(snapshot, range) {
                for style in styles(d) {
                    marks.push(Mark {
                        range: range.clone(),
                        severity: d.severity,
                        style,
                    });
                }
            }
            if d.severity == Severity::Hint {
                continue;
            }
            let info = rows.entry(row).or_insert(RowInfo {
                row,
                gutter: None,
                inline: None,
            });
            // `Severity` sorts `Error` first: smaller is more severe.
            if info.gutter.is_none_or(|s| d.severity < s) {
                info.gutter = Some(d.severity);
            }
            if info.inline.as_ref().is_none_or(|(s, _)| d.severity < *s) {
                info.inline = Some((d.severity, first_line(&d.message)));
            }
        }
        marks.sort_by_key(|m| (m.range.start.row, m.range.start.column));
        let rows: Vec<RowInfo> = rows.into_values().collect();
        let max_span = marks
            .iter()
            .map(|m| m.range.end.row - m.range.start.row)
            .max()
            .unwrap_or(0);
        let scrollbar = rows
            .iter()
            .filter_map(|r| Some((r.row, r.gutter?)))
            .filter(|(_, s)| *s <= Severity::Warning)
            .collect();
        DiagnosticRows {
            marks,
            max_span,
            rows,
            scrollbar,
        }
    }

    /// The marks touching any of `rows`.
    pub fn marks_in(&self, rows: Range<u32>) -> impl Iterator<Item = &Mark> {
        let lowest = rows.start.saturating_sub(self.max_span);
        let first = self.marks.partition_point(|m| m.range.start.row < lowest);
        self.marks[first..]
            .iter()
            .take_while(move |m| m.range.start.row < rows.end)
            .filter(move |m| m.range.end.row >= rows.start)
    }

    /// The icon of `row`: the highest of Error, Warning and Information
    /// starting on it; hints have none.
    pub fn gutter(&self, row: u32) -> Option<Severity> {
        self.row(row)?.gutter
    }

    /// The first line of the message of the most severe non-hint
    /// diagnostic starting on `row` (the first of equals).
    pub fn inline(&self, row: u32) -> Option<(Severity, &str)> {
        let (severity, message) = self.row(row)?.inline.as_ref()?;
        Some((*severity, message))
    }

    /// The rows with an error or warning, and the highest of the two.
    pub fn scrollbar(&self) -> &[(u32, Severity)] {
        &self.scrollbar
    }

    fn row(&self, row: u32) -> Option<&RowInfo> {
        let i = self.rows.binary_search_by_key(&row, |r| r.row).ok()?;
        Some(&self.rows[i])
    }
}

/// How a diagnostic is drawn: `Deprecated` strikes through (and is still
/// underlined at Error and Warning); `Unnecessary` below Warning fades
/// instead of underlining.
fn styles(d: &Diagnostic) -> Vec<MarkStyle> {
    let serious = d.severity <= Severity::Warning;
    let mut styles = Vec::new();
    if d.deprecated {
        styles.push(MarkStyle::Strike);
    }
    if d.unnecessary && !serious {
        styles.push(MarkStyle::Fade);
    } else if !d.deprecated || serious {
        styles.push(MarkStyle::Wavy);
    }
    styles
}

/// `range` with both ends inside the text: rows past the end go to the
/// end of the last line, columns past a line's end to its end, and a
/// column inside a char back to its start.
fn clamp_range(snapshot: &BufferSnapshot, range: Range<Point>) -> Range<Point> {
    let start = clamp_point(snapshot, range.start);
    let end = clamp_point(snapshot, range.end);
    let end = if (end.row, end.column) < (start.row, start.column) {
        start
    } else {
        end
    };
    start..end
}

fn clamp_point(snapshot: &BufferSnapshot, p: Point) -> Point {
    let last = snapshot.line_count().saturating_sub(1);
    let p = if p.row > last {
        Point {
            row: last,
            column: u32::MAX,
        }
    } else {
        p
    };
    let line = snapshot.line(p.row);
    let mut column = (p.column as usize).min(line.len());
    while !line.is_char_boundary(column) {
        column -= 1;
    }
    Point {
        row: p.row,
        column: column as u32,
    }
}

/// An empty `range` covers the char at it, or else the line's last char;
/// `None` on an empty line. Non-empty ranges are kept.
fn widen_empty(snapshot: &BufferSnapshot, range: Range<Point>) -> Option<Range<Point>> {
    if range.start != range.end {
        return Some(range);
    }
    let p = range.start;
    let line = snapshot.line(p.row);
    let (start, c) = match line[p.column as usize..].chars().next() {
        Some(c) => (p.column as usize, c),
        None => line.char_indices().next_back()?,
    };
    Some(
        Point {
            row: p.row,
            column: start as u32,
        }..Point {
            row: p.row,
            column: (start + c.len_utf8()) as u32,
        },
    )
}

/// The first line of `message`, at most [`INLINE_MAX_CHARS`] chars.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or("");
    match line.char_indices().nth(INLINE_MAX_CHARS) {
        Some((cut, _)) => line[..cut].to_owned(),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_lsp::position::{PositionEncoding, range_from_lsp};
    use lucerna_text::Buffer;

    fn snapshot(text: &str) -> BufferSnapshot {
        Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot()
    }

    fn pt(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    fn diag(range: Range<Point>, severity: Severity, message: &str) -> Diagnostic {
        Diagnostic {
            range,
            severity,
            message: message.into(),
            source: None,
            code: None,
            code_href: None,
            unnecessary: false,
            deprecated: false,
        }
    }

    fn marks(rows: &DiagnosticRows) -> Vec<Mark> {
        rows.marks_in(0..u32::MAX).cloned().collect()
    }

    #[test]
    fn empty_range_widens_to_one_char() {
        let text = snapshot("let x = 1;\nfoo\n\n");
        let rows = DiagnosticRows::build(
            &[
                // On `x`.
                diag(pt(0, 4)..pt(0, 4), Severity::Error, "a"),
                // At the end of `foo`: its last char.
                diag(pt(1, 3)..pt(1, 3), Severity::Warning, "b"),
                // On an empty line: nothing to underline.
                diag(pt(2, 0)..pt(2, 0), Severity::Error, "c"),
            ],
            &text,
        );
        assert_eq!(
            marks(&rows),
            vec![
                Mark {
                    range: pt(0, 4)..pt(0, 5),
                    severity: Severity::Error,
                    style: MarkStyle::Wavy,
                },
                Mark {
                    range: pt(1, 2)..pt(1, 3),
                    severity: Severity::Warning,
                    style: MarkStyle::Wavy,
                },
            ]
        );
        // The empty line still gets its gutter icon.
        assert_eq!(rows.gutter(2), Some(Severity::Error));
    }

    #[test]
    fn empty_range_widens_over_a_multibyte_char() {
        let text = snapshot("é😀\n");
        let rows = DiagnosticRows::build(&[diag(pt(0, 2)..pt(0, 2), Severity::Error, "a")], &text);
        assert_eq!(marks(&rows)[0].range, pt(0, 2)..pt(0, 6));
    }

    #[test]
    fn unnecessary_hint_fades() {
        let text = snapshot("use std::fmt;\nold();\nold();\nx\n");
        let mut unused = diag(pt(0, 4)..pt(0, 12), Severity::Hint, "unused");
        unused.unnecessary = true;
        let mut unused_warning = diag(pt(3, 0)..pt(3, 1), Severity::Warning, "unused var");
        unused_warning.unnecessary = true;
        let mut deprecated_hint = diag(pt(1, 0)..pt(1, 3), Severity::Hint, "deprecated");
        deprecated_hint.deprecated = true;
        let mut deprecated_error = diag(pt(2, 0)..pt(2, 3), Severity::Error, "deprecated");
        deprecated_error.deprecated = true;
        let rows = DiagnosticRows::build(
            &[unused, deprecated_hint, deprecated_error, unused_warning],
            &text,
        );
        let styles: Vec<(u32, MarkStyle)> = marks(&rows)
            .iter()
            .map(|m| (m.range.start.row, m.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (0, MarkStyle::Fade),
                (1, MarkStyle::Strike),
                (2, MarkStyle::Strike),
                (2, MarkStyle::Wavy),
                // Warning and above stay underlined.
                (3, MarkStyle::Wavy),
            ]
        );
    }

    #[test]
    fn gutter_takes_highest_severity_and_skips_hint() {
        let text = snapshot("a b c\nd\ne\n");
        let rows = DiagnosticRows::build(
            &[
                diag(pt(0, 0)..pt(0, 1), Severity::Information, "i"),
                diag(pt(0, 2)..pt(0, 3), Severity::Error, "e"),
                diag(pt(0, 4)..pt(0, 5), Severity::Warning, "w"),
                diag(pt(1, 0)..pt(1, 1), Severity::Hint, "h"),
                diag(pt(2, 0)..pt(2, 1), Severity::Information, "i2"),
            ],
            &text,
        );
        assert_eq!(rows.gutter(0), Some(Severity::Error));
        assert_eq!(rows.gutter(1), None);
        assert_eq!(rows.gutter(2), Some(Severity::Information));
        assert_eq!(rows.gutter(3), None);
        // Only errors and warnings, one per row.
        assert_eq!(rows.scrollbar(), &[(0, Severity::Error)]);
    }

    #[test]
    fn inline_uses_first_line() {
        let text = snapshot("a b\nc\n");
        let rows = DiagnosticRows::build(
            &[
                diag(pt(0, 0)..pt(0, 1), Severity::Warning, "warn first\nmore"),
                diag(
                    pt(0, 2)..pt(0, 3),
                    Severity::Error,
                    "mismatched types\nexpected `u32`",
                ),
                diag(pt(1, 0)..pt(1, 1), Severity::Hint, "just a hint"),
            ],
            &text,
        );
        assert_eq!(rows.inline(0), Some((Severity::Error, "mismatched types")));
        assert_eq!(rows.inline(1), None, "no inline message for hints");
    }

    #[test]
    fn emoji_line_mark_columns() {
        let text = snapshot("let 😀 = x\n");
        // UTF-16: `let ` is 4 units, the emoji 2, the space 1: `=` is 7..8
        // (bytes 9..10: the emoji is 4 bytes).
        let range = range_from_lsp(
            &text,
            lsp_types::Range {
                start: lsp_types::Position::new(0, 7),
                end: lsp_types::Position::new(0, 8),
            },
            PositionEncoding::Utf16,
        );
        let rows = DiagnosticRows::build(&[diag(range, Severity::Error, "e")], &text);
        let m = &marks(&rows)[0];
        assert_eq!(m.range, pt(0, 9)..pt(0, 10));
        assert_eq!(&text.line(0)[9..10], "=");
    }

    #[test]
    fn ranges_past_the_text_are_clamped() {
        // As after a reload that shortened the file, before the server
        // publishes again.
        let text = snapshot("ab😀\nxy\n");
        let rows = DiagnosticRows::build(
            &[
                // Ends inside the emoji (byte 3 of 4) and past the line.
                diag(pt(0, 1)..pt(0, 5), Severity::Error, "a"),
                // Rows past the end.
                diag(pt(7, 2)..pt(9, 4), Severity::Warning, "b"),
                // Past the line end of row 1.
                diag(pt(1, 10)..pt(1, 20), Severity::Error, "c"),
            ],
            &text,
        );
        let ranges: Vec<Range<Point>> = marks(&rows).into_iter().map(|m| m.range).collect();
        assert_eq!(
            ranges,
            vec![pt(0, 1)..pt(0, 2), pt(1, 1)..pt(1, 2), pt(1, 1)..pt(1, 2)]
        );
        for r in &ranges {
            let line = text.line(r.start.row);
            assert!(line.is_char_boundary(r.start.column as usize));
            assert!(line.is_char_boundary(r.end.column as usize));
        }
        assert_eq!(rows.gutter(1), Some(Severity::Error));
        assert_eq!(rows.gutter(7), None);
        assert_eq!(
            rows.scrollbar(),
            &[(0, Severity::Error), (1, Severity::Error)]
        );
    }

    #[test]
    fn many_diagnostics_group_by_row() {
        // 10 000 rows, two diagnostics each: an error then a warning on
        // even rows, a hint then an info on odd ones.
        let text = snapshot(&"abc\n".repeat(10_000));
        let mut diags = Vec::new();
        for row in 0..10_000u32 {
            let (a, b) = if row % 2 == 0 {
                (Severity::Error, Severity::Warning)
            } else {
                (Severity::Hint, Severity::Information)
            };
            diags.push(diag(pt(row, 0)..pt(row, 1), a, &format!("a{row}")));
            diags.push(diag(pt(row, 1)..pt(row, 2), b, &format!("b{row}")));
        }
        assert_eq!(diags.len(), 20_000);
        let rows = DiagnosticRows::build(&diags, &text);
        assert_eq!(rows.marks_in(0..u32::MAX).count(), 20_000);
        assert_eq!(rows.gutter(0), Some(Severity::Error));
        assert_eq!(rows.gutter(1), Some(Severity::Information));
        assert_eq!(rows.inline(9_998), Some((Severity::Error, "a9998")));
        assert_eq!(rows.inline(9_999), Some((Severity::Information, "b9999")));
        assert_eq!(rows.scrollbar().len(), 5_000);
        assert!(
            rows.scrollbar()
                .iter()
                .all(|&(r, s)| r % 2 == 0 && s == Severity::Error)
        );
        assert_eq!(rows.marks_in(5_000..5_001).count(), 2);
    }

    #[test]
    fn marks_in_finds_marks_spanning_into_rows() {
        let text = snapshot("a\nb\nc\nd\ne\n");
        let rows = DiagnosticRows::build(
            &[
                diag(pt(0, 0)..pt(3, 1), Severity::Error, "long"),
                diag(pt(4, 0)..pt(4, 1), Severity::Error, "last"),
            ],
            &text,
        );
        let found: Vec<&str> = rows
            .marks_in(2..3)
            .map(|m| {
                if m.range.start.row == 0 {
                    "long"
                } else {
                    "last"
                }
            })
            .collect();
        assert_eq!(found, vec!["long"]);
        assert_eq!(rows.marks_in(4..5).count(), 1);
        assert_eq!(rows.marks_in(5..9).count(), 0);
    }
}
