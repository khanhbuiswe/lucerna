use std::cmp::Reverse;
use std::ops::{ControlFlow, Range};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lucerna_text::{BufferSnapshot, Patch, PatchEdit};
use streaming_iterator::StreamingIterator;
use tree_sitter::{
    InputEdit, Node, ParseOptions, ParseState, Parser, QueryCursor, TextProvider, Tree,
};

use crate::brackets;
use crate::injection::{self, LayerParsers, Layers};
use crate::registry::{GrammarError, HighlightQuery, Language, LanguageRegistry, Precedence};
use crate::row_start;
use crate::symbols::SymbolTree;
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

/// The parse tree of one document plus the parser that produced it, and
/// the layers injected into it (`injections.scm`).
///
/// `Send` and `Sync`: the pipeline moves it to a background thread to parse,
/// and `parse` takes `&mut self`, so shared references only read the tree.
pub struct SyntaxState {
    language: Arc<Language>,
    /// Resolves the names of injected languages.
    registry: Arc<LanguageRegistry>,
    parser: Parser,
    tree: Option<Tree>,
    /// Child layers; empty until a parse.
    layers: Layers,
    layer_parsers: LayerParsers,
}

impl Drop for SyntaxState {
    fn drop(&mut self) {
        if let Some(store) = self.parser.take_wasm_store() {
            wasm::return_store(store);
        }
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SyntaxState>();
};

impl SyntaxState {
    /// Fails when the language's grammar cannot be loaded; callers then show
    /// the text unhighlighted.
    pub fn new(
        language: Arc<Language>,
        registry: Arc<LanguageRegistry>,
    ) -> Result<SyntaxState, GrammarError> {
        let grammar = language.grammar()?;
        let mut parser = Parser::new();
        if language.is_wasm() {
            parser
                .set_wasm_store(wasm::take_store()?)
                .map_err(|e| GrammarError(format!("wasm store: {e}")))?;
        }
        parser.set_language(&grammar).map_err(|e| {
            language.mark_failed();
            GrammarError(format!("grammar for {} is incompatible: {e}", language.id))
        })?;
        Ok(SyntaxState {
            language,
            registry,
            parser,
            tree: None,
            layers: Layers::default(),
            layer_parsers: LayerParsers::default(),
        })
    }

    /// The byte ranges of each child layer, for tests.
    #[doc(hidden)]
    pub fn layer_ranges(&self) -> Vec<Vec<Range<usize>>> {
        self.layers
            .iter()
            .map(|l| l.ranges.iter().map(|r| r.start_byte..r.end_byte).collect())
            .collect()
    }

    /// How many child layers (the root excluded) the last parse built.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// The depth of the deepest layer; the root is 0.
    #[doc(hidden)]
    pub fn max_layer_depth(&self) -> usize {
        self.layers.iter().map(|l| l.depth).max().unwrap_or(0)
    }

    pub fn language(&self) -> &Arc<Language> {
        &self.language
    }

    /// The symbols the language's `outline.scm` finds in the current tree
    /// (spec §3.2); `None` without a tree or without an outline query.
    /// Runs the query over the whole tree, without reparsing: call it off
    /// the UI thread.
    pub fn outline(&self, snapshot: &BufferSnapshot) -> Option<SymbolTree> {
        let tree = self.tree.as_ref()?;
        let query = self.language.queries()?.outline.as_ref()?;
        Some(query.symbols(tree, snapshot))
    }

    /// The current tree, if a parse has completed.
    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// Fully reparses `snapshot`, then rebuilds the injected layers. With
    /// `budget`, gives up once that much time has passed in all: a root
    /// parse that runs out keeps the previous tree and layers; a child parse
    /// that runs out keeps the new root and the layers built so far.
    pub fn parse(&mut self, snapshot: &BufferSnapshot, budget: Option<Duration>) -> ParseOutcome {
        let limit = Limit {
            deadline: budget.map(|budget| Instant::now() + budget),
            cancel: None,
        };
        self.parse_bounded(snapshot, false, limit, None)
    }

    /// Like [`SyntaxState::parse`], but incremental: the current tree is
    /// given to tree-sitter as the old tree, so the parts of it no edit
    /// touched are reused. The tree must describe `snapshot` once edited:
    /// it comes from [`SyntaxState::edited`] (or [`SyntaxState::fork`] of
    /// one), or from a parse of this same text. Without a tree it is a
    /// full parse. Setting `cancel` stops the parse as if out of time, so
    /// a newer edit can abandon a background parse of older text.
    pub fn reparse(
        &mut self,
        snapshot: &BufferSnapshot,
        budget: Option<Duration>,
        cancel: Option<&AtomicBool>,
    ) -> ParseOutcome {
        let limit = Limit {
            deadline: budget.map(|budget| Instant::now() + budget),
            cancel,
        };
        self.parse_bounded(snapshot, true, limit, None)
    }

    /// A new state (with its own parsers) holding the same tree and
    /// injected layers: what [`SyntaxState::reparse`] starts from while
    /// this one stays on screen.
    pub fn fork(&self) -> SyntaxState {
        let mut fork = SyntaxState::new(self.language.clone(), self.registry.clone())
            .expect("the language of an existing state loaded already");
        fork.tree = self.tree.clone();
        fork.layers = self.layers.clone();
        fork
    }

    /// Like an unbudgeted [`SyntaxState::parse`], but the layer build stops
    /// as if out of time once `max_children` layers are built: a
    /// deterministic stand-in for the time budget, for tests.
    #[doc(hidden)]
    pub fn parse_with_child_limit(
        &mut self,
        snapshot: &BufferSnapshot,
        max_children: usize,
    ) -> ParseOutcome {
        self.parse_bounded(snapshot, false, Limit::default(), Some(max_children))
    }

    /// With `incremental`, the current tree is the old tree of the parse.
    fn parse_bounded(
        &mut self,
        snapshot: &BufferSnapshot,
        incremental: bool,
        limit: Limit,
        max_children: Option<usize>,
    ) -> ParseOutcome {
        let old = if incremental {
            self.tree.as_ref()
        } else {
            None
        };
        let Some(tree) = parse_snapshot(&mut self.parser, snapshot, old, limit) else {
            return ParseOutcome::TimedOut;
        };
        let (layers, outcome) = injection::build_layers(
            &self.language,
            &tree,
            snapshot,
            &self.registry,
            &mut self.layer_parsers,
            limit,
            max_children,
            incremental.then_some(&self.layers),
        );
        self.layers = layers;
        self.tree = Some(tree);
        outcome
    }

    /// A copy for the edited text `new`, with the tree edited by
    /// `patch` (the change from `old`). Until `new` is parsed, it keeps
    /// the old colours on screen: ranges after the change shift, and those
    /// inside it stretch over the new text. Without a tree, the copy has
    /// none either. Injected layers are edited the same way (see
    /// `Layers::edited`), so they keep their colours too, and the next
    /// incremental parse reuses their trees.
    pub fn edited(&self, old: &BufferSnapshot, new: &BufferSnapshot, patch: &Patch) -> SyntaxState {
        let mut edited = SyntaxState::new(self.language.clone(), self.registry.clone())
            .expect("the language of an existing state loaded already");
        let Some(mut tree) = self.tree.clone() else {
            return edited;
        };
        // Too costly to edit: plain text until parsed.
        let root_children = tree.root_node().child_count() as usize;
        let Some(edits) = input_edits(old, new, patch, root_children) else {
            return edited;
        };
        // The estimate cannot see a tree's shape (an error tree is costlier
        // per edit): past the time budget, drop the tree too.
        let deadline = Instant::now() + EDIT_TIME_BUDGET;
        for (i, edit) in edits.iter().enumerate() {
            tree.edit(edit);
            if i % 8 == 7 && Instant::now() > deadline {
                return edited;
            }
        }
        edited.tree = Some(tree);
        edited.layers = self.layers.edited(&edits, deadline);
        edited
    }

    /// Spans for `rows` (end exclusive), clipped to those rows and sorted by
    /// start, outer spans before inner ones. Every layer with text in the
    /// rows contributes. Where several patterns capture the same range, only
    /// one survives: the deepest layer's, then the one its query intends.
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
        for segment in segments(snapshot, rows) {
            collect(
                &mut cursor,
                &queries.highlights,
                root,
                0,
                &segment,
                snapshot_text,
                styled,
                &mut spans,
            );
            for layer in self.layers.intersecting(&segment) {
                let Some(queries) = layer.language.queries() else {
                    continue;
                };
                collect(
                    &mut cursor,
                    &queries.highlights,
                    layer.tree.root_node(),
                    layer.depth,
                    &segment,
                    snapshot_text,
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
            Some(tree) => match self
                .language
                .queries()
                .and_then(|queries| queries.brackets.as_ref())
            {
                Some(query) => brackets::matching_bracket_by_query(tree, query, snapshot, offset),
                None => brackets::matching_bracket_in_tree(tree, offset),
            }
            .filter(|&at| at < snapshot.len_bytes()),
            None => brackets::matching_bracket(snapshot, offset),
        }
    }
}

struct RankedSpan {
    range: Range<usize>,
    /// The layer depth; the root is 0.
    depth: usize,
    /// Lower wins among spans with the same range and depth.
    rank: usize,
    capture: Arc<str>,
}

/// Runs `hq` over `range` of `node`'s tree (a layer at `depth`) and pushes
/// the captures, clipped to `range`.
#[allow(clippy::too_many_arguments)]
fn collect<I: AsRef<[u8]>>(
    cursor: &mut QueryCursor,
    hq: &HighlightQuery,
    node: Node,
    depth: usize,
    range: &Range<usize>,
    text: impl TextProvider<I>,
    styled: &dyn Fn(&str) -> bool,
    out: &mut Vec<RankedSpan>,
) {
    cursor.set_byte_range(range.clone());
    let mut captures = cursor.captures(&hq.query, node, text);
    while let Some((m, index)) = captures.next() {
        if !hq.accepts(m) {
            continue;
        }
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
            range: clipped,
            depth,
            rank,
            capture: name.clone(),
        });
    }
}

/// Sorts by start, outer spans first; of spans with the same range, the
/// deepest layer's best-ranked one survives.
fn resolve(mut spans: Vec<RankedSpan>) -> Vec<HighlightSpan> {
    spans.sort_by_key(|s| {
        (
            s.range.start,
            Reverse(s.range.end),
            Reverse(s.depth),
            s.rank,
        )
    });
    spans.dedup_by(|later, kept| later.range == kept.range);
    spans
        .into_iter()
        .map(|s| HighlightSpan {
            range: s.range,
            capture: s.capture,
        })
        .collect()
}

/// When a parse gives up: past a deadline, or once a cancel flag is set.
#[derive(Clone, Copy, Default)]
pub(crate) struct Limit<'a> {
    pub(crate) deadline: Option<Instant>,
    pub(crate) cancel: Option<&'a AtomicBool>,
}

impl Limit<'_> {
    /// Whether the parse may give up at all.
    pub(crate) fn is_bounded(&self) -> bool {
        self.deadline.is_some() || self.cancel.is_some()
    }

    pub(crate) fn reached(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
            || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// Parses all of `snapshot` (within the parser's included ranges), reusing
/// `old` (already edited to the text) where nothing changed. `None` once
/// `limit` is reached; the parser is then reset, so the next call starts
/// afresh rather than resuming.
pub(crate) fn parse_snapshot(
    parser: &mut Parser,
    snapshot: &BufferSnapshot,
    old: Option<&Tree>,
    limit: Limit,
) -> Option<Tree> {
    let len = snapshot.len_bytes();
    let mut read = |offset: usize, _: tree_sitter::Point| -> &str {
        if offset >= len {
            ""
        } else {
            snapshot.chunks_in_range(offset..len).next().unwrap_or("")
        }
    };
    // Small parses may finish before the first progress callback.
    if limit.reached() {
        return None;
    }
    let tree = if limit.is_bounded() {
        let mut progress = |_: &ParseState| {
            if limit.reached() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let options = ParseOptions::new().progress_callback(&mut progress);
        parser.parse_with_options(&mut read, old, Some(options))
    } else {
        parser.parse_with_options(&mut read, old, None)
    };
    if tree.is_none() {
        parser.reset();
    }
    tree
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

/// Roughly how many tree nodes the edits after one change may visit on
/// the UI thread. tree-sitter scans a node's children from the first one
/// for every edit, and visits every node inside an edit, so 1,000 cursors
/// in a file of 10,000 top-level items cost 10 million visits (hundreds of
/// ms); the budget is a few ms. Past the budget the tree is dropped: plain text until the parse
/// (in the background) lands.
const EDIT_BUDGET: usize = 100_000;

/// The most time editing the trees after one change may take on the UI
/// thread before they are dropped (see [`EDIT_BUDGET`]).
const EDIT_TIME_BUDGET: Duration = Duration::from_millis(2);

/// Edits this close (in bytes) merge into one: the text between them counts
/// as replaced, which costs a visit of its few nodes rather than a scan
/// from the first child per edit (a cursor on each of many lines).
const MERGE_GAP: usize = 256;

/// `patch`'s edits, or, when applying them one by one to a tree whose root
/// has `root_children` children would exceed [`EDIT_BUDGET`], with those
/// [`MERGE_GAP`] apart or closer merged into one. `None` when even that
/// exceeds it.
fn grouped(patch: &Patch, root_children: usize) -> Option<Vec<PatchEdit>> {
    // A scan of the root's children per edit, and a node per few bytes
    // inside the edits.
    let cost = |edits: &[PatchEdit]| {
        let spanned: usize = edits.iter().map(|e| e.old.len() + e.new.len()).sum();
        edits.len() * (root_children + 64) + spanned / 4
    };
    if cost(patch.edits()) <= EDIT_BUDGET {
        return Some(patch.edits().to_vec());
    }
    let mut out: Vec<PatchEdit> = Vec::new();
    for edit in patch.edits() {
        match out.last_mut() {
            Some(last) if edit.old.start - last.old.end <= MERGE_GAP => {
                last.old.end = edit.old.end;
                last.new.end = edit.new.end;
            }
            _ => out.push(edit.clone()),
        }
    }
    (cost(&out) <= EDIT_BUDGET).then_some(out)
}

/// The tree-sitter edits for `patch` (merged as [`grouped`] does), to
/// apply in order, or `None` when they would cost too much. They run from
/// the end of the text to the start, so each is expressed in the offsets
/// of the text the earlier ones left alone.
fn input_edits(
    old: &BufferSnapshot,
    new: &BufferSnapshot,
    patch: &Patch,
    root_children: usize,
) -> Option<Vec<InputEdit>> {
    let point = |snapshot: &BufferSnapshot, offset: usize| {
        let p = snapshot.offset_to_point(offset);
        tree_sitter::Point::new(p.row as usize, p.column as usize)
    };
    let edits = grouped(patch, root_children)?;
    Some(
        edits
            .iter()
            .rev()
            .map(|edit| {
                let start_position = point(old, edit.old.start);
                let (from, to) = (point(new, edit.new.start), point(new, edit.new.end));
                // Relative to the start: the text before it is still old.
                let new_end_position = if to.row == from.row {
                    tree_sitter::Point::new(
                        start_position.row,
                        start_position.column + to.column - from.column,
                    )
                } else {
                    tree_sitter::Point::new(start_position.row + to.row - from.row, to.column)
                };
                InputEdit {
                    start_byte: edit.old.start,
                    old_end_byte: edit.old.end,
                    new_end_byte: edit.old.start + edit.new.len(),
                    start_position,
                    old_end_position: point(old, edit.old.end),
                    new_end_position,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::{Buffer, PatchEdit, decode_file, diff_edits};

    fn edits_for(old_text: &str, new_text: &str) -> Vec<InputEdit> {
        let mut buffer = Buffer::from_bytes(old_text.as_bytes(), false).unwrap();
        let old = buffer.snapshot();
        let decoded = decode_file(new_text.as_bytes(), false).unwrap();
        let patch = buffer.edit(diff_edits(&old, &decoded.text));
        let new = buffer.snapshot();
        input_edits(&old, &new, &patch, 0).expect("within budget")
    }

    #[test]
    fn edits_are_valid_and_account_for_every_byte() {
        let lf = "fn a() {}\nfn b() {}\n";
        for (old, new) in [
            (lf, "fn a() {}\nfn b() {}"),
            ("fn a() {}\nfn b() {}", lf),
            (lf, "fn a() {}\n"),
            ("", lf),
            ("a\nb\nc\n", "a\nX\nc\n"),
            ("a", "a\nb"),
        ] {
            let edits = edits_for(old, new);
            let mut delta = 0isize;
            for e in &edits {
                let ctx = format!("{old:?} -> {new:?}: {e:?}");
                assert!(e.start_byte <= e.old_end_byte, "{ctx}");
                assert!(e.start_byte <= e.new_end_byte, "{ctx}");
                assert!(e.start_position <= e.old_end_position, "{ctx}");
                assert!(e.start_position <= e.new_end_position, "{ctx}");
                assert!(e.old_end_byte <= old.len(), "{ctx}");
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
    fn edits_run_from_the_end_of_the_text_to_the_start() {
        let old = Buffer::from_bytes(b"abcdef", false).unwrap().snapshot();
        let new = Buffer::from_bytes(b"aXbcdYYef", false).unwrap().snapshot();
        let patch = Patch::new(vec![
            PatchEdit {
                old: 1..1,
                new: 1..2,
            },
            PatchEdit {
                old: 4..4,
                new: 5..7,
            },
        ]);
        let edits = input_edits(&old, &new, &patch, 0).expect("within budget");
        assert_eq!(
            edits
                .iter()
                .map(|e| (e.start_byte, e.new_end_byte))
                .collect::<Vec<_>>(),
            vec![(4, 6), (1, 2)]
        );
    }

    #[test]
    fn close_edits_merge() {
        let text = "x\n".repeat(1000);
        let snapshot = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        // Empty edits at 0, 2, 4, then 1000 and 1002: two clusters.
        let at = [0usize, 2, 4, 1000, 1002];
        let patch = Patch::new(
            at.iter()
                .map(|&i| PatchEdit {
                    old: i..i,
                    new: i..i,
                })
                .collect(),
        );
        // Within the budget, every edit stays.
        assert_eq!(
            input_edits(&snapshot, &snapshot, &patch, 0).unwrap().len(),
            5
        );
        // Over it, close ones merge.
        let edits = input_edits(&snapshot, &snapshot, &patch, 30_000).unwrap();
        assert_eq!(
            edits
                .iter()
                .map(|e| (e.start_byte, e.old_end_byte, e.new_end_byte))
                .collect::<Vec<_>>(),
            vec![(1000, 1002, 1002), (0, 4, 4)]
        );
        assert_eq!(edits[1].old_end_position, tree_sitter::Point::new(2, 0));
    }

    #[test]
    fn edits_past_the_budget_give_up() {
        let text = "x\n".repeat(160_000);
        let snapshot = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        // 1,000 cursors, too far apart to merge.
        let patch = Patch::new(
            (0..1000)
                .map(|i| PatchEdit {
                    old: 300 * i..300 * i,
                    new: 300 * i..300 * i,
                })
                .collect(),
        );
        assert!(input_edits(&snapshot, &snapshot, &patch, 100).is_none());
        let few = Patch::new(patch.edits()[..10].to_vec());
        assert_eq!(
            input_edits(&snapshot, &snapshot, &few, 100).unwrap().len(),
            10
        );
        // A large root makes even a few edits costly.
        assert!(input_edits(&snapshot, &snapshot, &few, 20_000).is_none());
    }
}
