use std::cmp::Reverse;
use std::ops::{ControlFlow, Range};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lucerna_text::{BufferSnapshot, Reload};
use streaming_iterator::StreamingIterator;
use tree_sitter::{
    InputEdit, Node, ParseOptions, ParseState, Parser, QueryCursor, TextProvider, Tree,
};

use crate::brackets;
use crate::registry::{GrammarError, HighlightQuery, Language, Precedence};
use crate::row_start;
use crate::wasm;

/// How long the document pipeline parses synchronously before handing the
/// parse to a background thread.
pub const SYNC_PARSE_BUDGET: Duration = Duration::from_millis(1);

/// A line longer than this many chars only gets spans for its first part.
const MAX_HIGHLIGHTED_LINE_CHARS: usize = 10_000;

/// One highlighted range. `range` is in absolute bytes; `capture` is the
/// query's capture name (e.g. `keyword.function`), shared per language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub capture: Arc<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseOutcome {
    Done,
    /// The budget ran out; the previous tree (if any) is kept.
    TimedOut,
}

/// The parse tree of one document plus the parser that produced it.
///
/// `Send` and `Sync`: the pipeline moves it to a background thread to parse,
/// and `parse` takes `&mut self`, so shared references only read the tree.
pub struct SyntaxState {
    language: Arc<Language>,
    parser: Parser,
    tree: Option<Tree>,
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SyntaxState>();
};

impl SyntaxState {
    /// Fails when the language's grammar cannot be loaded; callers then show
    /// the text unhighlighted.
    pub fn new(language: Arc<Language>) -> Result<SyntaxState, GrammarError> {
        let grammar = language.grammar()?;
        let mut parser = Parser::new();
        if language.is_wasm() {
            parser
                .set_wasm_store(wasm::new_store()?)
                .map_err(|e| GrammarError(format!("wasm store: {e}")))?;
        }
        parser.set_language(&grammar).map_err(|e| {
            language.mark_failed();
            GrammarError(format!("grammar for {} is incompatible: {e}", language.id))
        })?;
        Ok(SyntaxState {
            language,
            parser,
            tree: None,
        })
    }

    /// The current tree, if a parse has completed.
    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// Fully reparses `snapshot`. With `budget`, gives up once that much
    /// time has passed and keeps the previous tree.
    pub fn parse(&mut self, snapshot: &BufferSnapshot, budget: Option<Duration>) -> ParseOutcome {
        let len = snapshot.len_bytes();
        let mut read = |offset: usize, _: tree_sitter::Point| -> &str {
            if offset >= len {
                ""
            } else {
                snapshot.chunks_in_range(offset..len).next().unwrap_or("")
            }
        };
        let tree = match budget {
            None => self.parser.parse_with_options(&mut read, None, None),
            Some(budget) => {
                let started = Instant::now();
                let mut progress = |_: &ParseState| {
                    if started.elapsed() >= budget {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                };
                let options = ParseOptions::new().progress_callback(&mut progress);
                self.parser
                    .parse_with_options(&mut read, None, Some(options))
            }
        };
        match tree {
            Some(tree) => {
                self.tree = Some(tree);
                ParseOutcome::Done
            }
            None => {
                // A cancelled parse would otherwise resume on the next call.
                self.parser.reset();
                ParseOutcome::TimedOut
            }
        }
    }

    /// A copy for the reloaded text `new`, with the tree edited by
    /// `reload` (the change from `old`). Until `new` is parsed, it keeps
    /// the old colours on screen: ranges after the change shift, and those
    /// inside it stretch over the new text. Without a tree, the copy has
    /// none either.
    pub fn edited(
        &self,
        old: &BufferSnapshot,
        new: &BufferSnapshot,
        reload: &Reload,
    ) -> SyntaxState {
        let mut edited = SyntaxState::new(self.language.clone())
            .expect("the language of an existing state loaded already");
        edited.tree = self.tree.clone().and_then(|mut tree| {
            // Too many changed line endings: show plain text until parsed.
            let edits = input_edits(old, new, reload)?;
            for edit in &edits {
                tree.edit(edit);
            }
            Some(tree)
        });
        edited
    }

    /// Spans for `rows` (end exclusive), clipped to those rows and sorted by
    /// start, outer spans before inner ones. Where several patterns capture
    /// the same range, only the one the language's query intends survives.
    ///
    /// Captures for which `styled` is false are dropped before ranking, so
    /// a less specific capture the theme styles can show through.
    pub fn highlights(
        &self,
        snapshot: &BufferSnapshot,
        rows: Range<u32>,
        styled: &dyn Fn(&str) -> bool,
    ) -> Vec<HighlightSpan> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let Some(queries) = self.language.queries() else {
            return Vec::new();
        };
        let root = tree.root_node();
        // An edited tree (see `edited`) may still hold a range past the
        // end of a shorter text; clamp rather than panic.
        let len = snapshot.len_bytes();
        let snapshot_text = move |node: Node| {
            let range = node.byte_range();
            snapshot
                .chunks_in_range(range.start.min(len)..range.end.min(len))
                .map(str::as_bytes)
        };
        let mut cursor = QueryCursor::new();
        let mut spans = Vec::new();
        let mut inline_parser = None;
        for segment in segments(snapshot, rows) {
            collect(
                &mut cursor,
                &queries.highlights,
                root,
                Layer::Main,
                &segment,
                0,
                snapshot_text,
                styled,
                &mut spans,
            );
            let Some(inline) = &queries.inline else {
                continue;
            };
            let mut inline_ranges = Vec::new();
            cursor.set_byte_range(segment.clone());
            let mut matches = cursor.matches(&inline.inline_nodes, root, snapshot_text);
            while let Some(m) = matches.next() {
                for capture in m.captures() {
                    let range = clip(capture.node.byte_range(), &segment);
                    if !range.is_empty() {
                        inline_ranges.push(range);
                    }
                }
            }
            for range in inline_ranges {
                let text = snapshot.text_for_range(range.clone());
                let parser = inline_parser.get_or_insert_with(|| {
                    let mut parser = Parser::new();
                    parser
                        .set_language(&inline.grammar)
                        .expect("bundled inline grammar is ABI-compatible");
                    parser
                });
                let Some(inline_tree) = parser.parse(&text, None) else {
                    continue;
                };
                collect(
                    &mut cursor,
                    &inline.highlights,
                    inline_tree.root_node(),
                    Layer::MarkdownInline,
                    &(0..text.len()),
                    range.start,
                    text.as_bytes(),
                    styled,
                    &mut spans,
                );
            }
        }
        resolve(spans)
    }

    /// Like [`crate::matching_bracket`], but uses the tree when there is one,
    /// so brackets inside strings and comments are ignored.
    pub fn matching_bracket(&self, snapshot: &BufferSnapshot, offset: usize) -> Option<usize> {
        match &self.tree {
            // An edited tree may name an offset past a shorter text.
            Some(tree) => brackets::matching_bracket_in_tree(tree, offset)
                .filter(|&at| at < snapshot.len_bytes()),
            None => brackets::matching_bracket(snapshot, offset),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    Main,
    MarkdownInline,
}

struct RankedSpan {
    range: Range<usize>,
    layer: Layer,
    /// Lower wins among spans with the same range and layer.
    rank: usize,
    capture: Arc<str>,
}

/// Runs `hq` over `range` of `node`'s tree (whose byte 0 is `offset` in the
/// document) and pushes the captures, clipped to `range`.
#[allow(clippy::too_many_arguments)]
fn collect<I: AsRef<[u8]>>(
    cursor: &mut QueryCursor,
    hq: &HighlightQuery,
    node: Node,
    layer: Layer,
    range: &Range<usize>,
    offset: usize,
    text: impl TextProvider<I>,
    styled: &dyn Fn(&str) -> bool,
    out: &mut Vec<RankedSpan>,
) {
    cursor.set_byte_range(range.clone());
    let mut captures = cursor.captures(&hq.query, node, text);
    while let Some((m, index)) = captures.next() {
        let capture = m.captures()[*index];
        let name = &hq.names[capture.index as usize];
        // Captures starting with `_` only feed predicates.
        if name.starts_with('_') || !styled(name) {
            continue;
        }
        let clipped = clip(capture.node.byte_range(), range);
        if clipped.is_empty() {
            continue;
        }
        let rank = match hq.precedence {
            Precedence::FirstPatternWins => m.pattern_index,
            Precedence::LastPatternWins => usize::MAX - m.pattern_index,
        };
        out.push(RankedSpan {
            range: clipped.start + offset..clipped.end + offset,
            layer,
            rank,
            capture: name.clone(),
        });
    }
}

fn resolve(mut spans: Vec<RankedSpan>) -> Vec<HighlightSpan> {
    spans.sort_by_key(|s| (s.range.start, Reverse(s.range.end), s.layer, s.rank));
    spans.dedup_by(|later, kept| later.range == kept.range && later.layer == kept.layer);
    spans
        .into_iter()
        .map(|s| HighlightSpan {
            range: s.range,
            capture: s.capture,
        })
        .collect()
}

fn clip(range: Range<usize>, to: &Range<usize>) -> Range<usize> {
    let start = range.start.max(to.start);
    start..range.end.min(to.end).max(start)
}

/// The byte ranges to highlight for `rows`: the rows' bytes, minus the tail
/// of every line longer than [`MAX_HIGHLIGHTED_LINE_CHARS`].
fn segments(snapshot: &BufferSnapshot, rows: Range<u32>) -> Vec<Range<usize>> {
    let last = rows.end.min(snapshot.line_count());
    let mut segments = Vec::new();
    if rows.start >= last {
        return segments;
    }
    let mut segment_start = row_start(snapshot, rows.start);
    for row in rows.start..last {
        let line = row_start(snapshot, row)..row_start(snapshot, row + 1);
        if line.len() <= MAX_HIGHLIGHTED_LINE_CHARS {
            continue;
        }
        if let Some(cap) = long_line_cap(snapshot, line.clone()) {
            segments.push(segment_start..cap);
            segment_start = line.end;
        }
    }
    segments.push(segment_start..row_start(snapshot, last));
    segments.retain(|s| !s.is_empty());
    segments
}

/// The byte offset just past the line's first [`MAX_HIGHLIGHTED_LINE_CHARS`]
/// chars, or `None` when the line (without its line ending) is not longer.
fn long_line_cap(snapshot: &BufferSnapshot, line: Range<usize>) -> Option<usize> {
    let mut chars = 0;
    let mut pos = line.start;
    for chunk in snapshot.chunks_in_range(line) {
        for (i, c) in chunk.char_indices() {
            if chars == MAX_HIGHLIGHTED_LINE_CHARS {
                return (c != '\n' && c != '\r').then_some(pos + i);
            }
            chars += 1;
        }
        pos += chunk.len();
    }
    None
}

/// Most line-ending edits [`input_edits`] makes before giving up.
const MAX_EOL_EDITS: usize = 1000;

/// The byte length of the line ending of `row`.
fn eol_len(snapshot: &BufferSnapshot, row: u32) -> usize {
    let (start, end) = (row_start(snapshot, row), row_start(snapshot, row + 1));
    if end > start && snapshot.byte_at(end - 1) == b'\n' {
        if end - 1 > start && snapshot.byte_at(end - 2) == b'\r' {
            2
        } else {
            1
        }
    } else {
        0
    }
}

/// The tree-sitter edits for `reload`, to apply in order, or `None` when
/// too many line endings changed to be worth editing (the caller drops
/// the tree). Each edit is valid (its start lies within both ends), and
/// they run from the end of the text to the start, so each is expressed
/// in the offsets of the text the earlier ones left alone.
///
/// Rows compare equal whatever their line ending, so besides the changed
/// rows there is one small edit per unchanged row whose line ending
/// changed length (LF to CRLF, R31), keeping the offsets of the head and
/// tail right until the text is parsed again.
fn input_edits(
    old: &BufferSnapshot,
    new: &BufferSnapshot,
    reload: &Reload,
) -> Option<Vec<InputEdit>> {
    let point = |snapshot: &BufferSnapshot, offset: usize| {
        let p = snapshot.offset_to_point(offset);
        tree_sitter::Point::new(p.row as usize, p.column as usize)
    };
    // The edit of the line ending of old row `old_row` / new row `new_row`.
    let eol_edit = |old_row: u32, new_row: u32| {
        let (old_eol, new_eol) = (eol_len(old, old_row), eol_len(new, new_row));
        if old_eol == new_eol {
            return None;
        }
        let start = row_start(old, old_row + 1) - old_eol;
        let start_position = point(old, start);
        Some(InputEdit {
            start_byte: start,
            old_end_byte: start + old_eol,
            new_end_byte: start + new_eol,
            start_position,
            old_end_position: point(old, start + old_eol),
            new_end_position: if new_eol > 0 {
                tree_sitter::Point::new(start_position.row + 1, 0)
            } else {
                start_position
            },
        })
    };
    let (sr, oe, ne) = (reload.start_row, reload.old_end_row, reload.new_end_row);
    let (main_start, new_main_start) = (row_start(old, sr), row_start(new, sr));
    let main_old = row_start(old, oe) - main_start;
    let main_new = row_start(new, ne) - new_main_start;
    // Same head length and same total outside the changed rows: no line
    // ending outside them changed length.
    let outside_unchanged =
        main_start == new_main_start && old.len_bytes() - main_old == new.len_bytes() - main_new;
    let mut edits = Vec::new();
    let mut eol_edits = Vec::new();
    if !outside_unchanged {
        for row in (oe..old.line_count()).rev() {
            eol_edits.extend(eol_edit(row, row - oe + ne));
            if eol_edits.len() > MAX_EOL_EDITS {
                return None;
            }
        }
    }
    edits.append(&mut eol_edits);
    if sr != oe || sr != ne {
        // Relative to the start, as the head still has its old endings.
        let (from, to) = (
            point(new, new_main_start),
            point(new, new_main_start + main_new),
        );
        let start_position = point(old, main_start);
        let new_end_position = if to.row == from.row {
            tree_sitter::Point::new(
                start_position.row,
                start_position.column + to.column - from.column,
            )
        } else {
            tree_sitter::Point::new(start_position.row + to.row - from.row, to.column)
        };
        edits.push(InputEdit {
            start_byte: main_start,
            old_end_byte: main_start + main_old,
            new_end_byte: main_start + main_new,
            start_position,
            old_end_position: point(old, main_start + main_old),
            new_end_position,
        });
    }
    if !outside_unchanged {
        for row in (0..sr).rev() {
            eol_edits.extend(eol_edit(row, row));
            if edits.len() + eol_edits.len() > MAX_EOL_EDITS {
                return None;
            }
        }
    }
    edits.append(&mut eol_edits);
    Some(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Buffer;

    fn edits_for(old_text: &str, new_text: &str) -> Vec<InputEdit> {
        let mut buffer = Buffer::from_bytes(old_text.as_bytes(), false).unwrap();
        let old = buffer.snapshot();
        let reload = buffer.reload(new_text.as_bytes()).unwrap();
        input_edits(&old, &buffer.snapshot(), &reload).expect("below the cap")
    }

    #[test]
    fn edits_are_valid_and_account_for_every_byte() {
        let lf = "fn a() {}\nfn b() {}\n";
        let crlf = lf.replace('\n', "\r\n");
        for (old, new) in [
            (crlf.as_str(), lf),
            (lf, crlf.as_str()),
            (lf, "fn a() {}\nfn b() {}"),
            ("fn a() {}\nfn b() {}", lf),
            (lf, "fn a() {}\n"),
            ("", lf),
            ("fn a() {}\r\nfn b() {}\n", "fn a() {}\nfn b() {}\r\n"),
            ("a\nb\nc\n", "a\r\nX\nc\n"),
            ("a", "a\nb"),
            ("a\r\nb\nc", "a\nX\nY\r\nc\r\n"),
        ] {
            let edits = edits_for(old, new);
            let mut delta = 0isize;
            let mut previous_start = usize::MAX;
            for e in &edits {
                let ctx = format!("{old:?} -> {new:?}: {e:?}");
                assert!(e.start_byte <= e.old_end_byte, "{ctx}");
                assert!(e.start_byte <= e.new_end_byte, "{ctx}");
                assert!(e.start_position <= e.old_end_position, "{ctx}");
                assert!(e.start_position <= e.new_end_position, "{ctx}");
                assert!(e.old_end_byte <= old.len(), "{ctx}");
                assert!(
                    e.start_byte <= previous_start,
                    "applied end to start: {ctx}"
                );
                previous_start = e.start_byte;
                delta += e.new_end_byte as isize - e.old_end_byte as isize;
            }
            assert_eq!(
                old.len() as isize + delta,
                new.len() as isize,
                "{old:?} -> {new:?}: {edits:?}"
            );
        }
    }

    #[test]
    fn too_many_changed_line_endings_give_up() {
        let lf = "x\n".repeat(MAX_EOL_EDITS + 1);
        let mut buffer = Buffer::from_bytes(lf.as_bytes(), false).unwrap();
        let old = buffer.snapshot();
        let reload = buffer.reload(lf.replace('\n', "\r\n").as_bytes()).unwrap();
        assert!(input_edits(&old, &buffer.snapshot(), &reload).is_none());
        let few = "x\n".repeat(MAX_EOL_EDITS);
        let mut buffer = Buffer::from_bytes(few.as_bytes(), false).unwrap();
        let old = buffer.snapshot();
        let reload = buffer.reload(few.replace('\n', "\r\n").as_bytes()).unwrap();
        let edits = input_edits(&old, &buffer.snapshot(), &reload).unwrap();
        assert_eq!(edits.len(), MAX_EOL_EDITS);
    }
}
