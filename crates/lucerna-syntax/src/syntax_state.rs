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
use crate::registry::{HighlightQuery, Language, Precedence};
use crate::row_start;

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
    pub fn new(language: Arc<Language>) -> SyntaxState {
        let mut parser = Parser::new();
        parser
            .set_language(language.grammar())
            .unwrap_or_else(|e| panic!("bundled grammar {} is ABI-compatible: {e}", language.id));
        SyntaxState {
            language,
            parser,
            tree: None,
        }
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
        let mut edited = SyntaxState::new(self.language.clone());
        edited.tree = self.tree.clone().map(|mut tree| {
            tree.edit(&input_edit(old, new, reload));
            tree
        });
        edited
    }

    /// Spans for `rows` (end exclusive), clipped to those rows and sorted by
    /// start, outer spans before inner ones. Where several patterns capture
    /// the same range, only the one the language's query intends survives.
    pub fn highlights(&self, snapshot: &BufferSnapshot, rows: Range<u32>) -> Vec<HighlightSpan> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let queries = self.language.queries();
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
    out: &mut Vec<RankedSpan>,
) {
    cursor.set_byte_range(range.clone());
    let mut captures = cursor.captures(&hq.query, node, text);
    while let Some((m, index)) = captures.next() {
        let capture = m.captures()[*index];
        let name = &hq.names[capture.index as usize];
        // Captures starting with `_` only feed predicates.
        if name.starts_with('_') {
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

/// The tree-sitter edit for `reload`. Its byte offsets and points are
/// consistent: the start never lies past either end (R30).
fn input_edit(old: &BufferSnapshot, new: &BufferSnapshot, reload: &Reload) -> InputEdit {
    let mut start = row_start(old, reload.start_row);
    let old_end = row_start(old, reload.old_end_row);
    let new_end = row_start(new, reload.new_end_row);
    // Rows that compare equal can still differ in bytes (CRLF to LF, a
    // removed final newline), leaving `start` past an end. Fall back to
    // a whole-file edit then.
    let whole_file = start > old_end || start > new_end;
    if whole_file {
        start = 0;
    }
    let (old_end, new_end) = if whole_file {
        (old.len_bytes(), new.len_bytes())
    } else {
        (old_end, new_end)
    };
    let point = |snapshot: &BufferSnapshot, offset: usize| {
        let p = snapshot.offset_to_point(offset);
        tree_sitter::Point::new(p.row as usize, p.column as usize)
    };
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(old, start),
        old_end_position: point(old, old_end),
        new_end_position: point(new, new_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Buffer;

    fn edit_for(old_text: &str, new_text: &str) -> InputEdit {
        let mut buffer = Buffer::from_bytes(old_text.as_bytes(), false).unwrap();
        let old = buffer.snapshot();
        let reload = buffer.reload(new_text.as_bytes()).unwrap();
        input_edit(&old, &buffer.snapshot(), &reload)
    }

    #[test]
    fn edits_never_start_past_their_ends() {
        let lf = "fn a() {}\nfn b() {}\n";
        let crlf = lf.replace('\n', "\r\n");
        for (old, new) in [
            (crlf.as_str(), lf),
            (lf, crlf.as_str()),
            (lf, "fn a() {}\nfn b() {}"),
            ("fn a() {}\nfn b() {}", lf),
            (lf, "fn a() {}\n"),
            ("", lf),
        ] {
            let e = edit_for(old, new);
            assert!(e.start_byte <= e.old_end_byte, "{old:?} -> {new:?}: {e:?}");
            assert!(e.start_byte <= e.new_end_byte, "{old:?} -> {new:?}: {e:?}");
            assert!(e.start_position <= e.old_end_position, "{e:?}");
            assert!(e.start_position <= e.new_end_position, "{e:?}");
            assert!(e.old_end_byte <= old.len(), "{e:?}");
            assert!(e.new_end_byte <= new.len(), "{e:?}");
        }
    }
}
