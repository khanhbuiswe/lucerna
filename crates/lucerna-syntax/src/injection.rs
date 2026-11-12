//! Injections: reading an `injections.scm` query, and the tree of layers
//! (one tree per injected region) that [`crate::SyntaxState`] keeps under
//! its root tree.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::{ControlFlow, Range};
use std::sync::Arc;
use std::time::Instant;

use lucerna_text::BufferSnapshot;
use streaming_iterator::StreamingIterator;
use tree_sitter::{
    InputEdit, Node, Parser, Query, QueryCursor, QueryCursorOptions, QueryCursorState, Tree,
};

use crate::registry::{Language, LanguageOrigin, LanguageRegistry, Predicates};
use crate::syntax_state::{Limit, ParseOutcome, parse_snapshot};
use crate::wasm;

/// The deepest a layer may be; the root is depth 0.
pub(crate) const MAX_DEPTH: usize = 4;

/// Most child layers in one document. Markdown's own inline layers do not
/// count (see [`uncapped`]).
pub(crate) const MAX_LAYERS: usize = 1000;

/// Longest injected language name read from the text, in bytes.
const MAX_NAME_LEN: usize = 128;

/// A compiled `injections.scm` with the captures and `#set!` properties it
/// uses resolved once.
pub(crate) struct InjectionQuery {
    query: Query,
    /// `@injection.content` / `@content`.
    content: Vec<u32>,
    /// `@injection.language` / `@language`.
    language: Vec<u32>,
    /// Per pattern.
    patterns: Vec<PatternSettings>,
    /// Unknown general predicates disable their pattern, as in highlights.
    predicates: Predicates,
}

#[derive(Default)]
struct PatternSettings {
    /// `#set! injection.language "<name>"`.
    language: Option<Box<str>>,
    /// `#set! injection.combined`.
    combined: bool,
}

impl InjectionQuery {
    /// `injection.include-children` and every other property are ignored:
    /// the content is always the whole node.
    pub(crate) fn new(query: Query, language_id: &str) -> InjectionQuery {
        let indices = |names: [&str; 2]| {
            names
                .iter()
                .filter_map(|n| query.capture_index_for_name(n))
                .collect::<Vec<_>>()
        };
        let content = indices(["injection.content", "content"]);
        let language = indices(["injection.language", "language"]);
        let patterns = (0..query.pattern_count())
            .map(|pattern| {
                let mut settings = PatternSettings::default();
                for property in query.property_settings(pattern) {
                    match &*property.key {
                        "injection.language" => settings.language = property.value.clone(),
                        "injection.combined" => settings.combined = true,
                        _ => {}
                    }
                }
                settings
            })
            .collect();
        let predicates = Predicates::new(&query, language_id);
        InjectionQuery {
            query,
            content,
            language,
            patterns,
            predicates,
        }
    }
}

/// A region to parse as the language named `language`.
struct Injection {
    language: String,
    ranges: Vec<tree_sitter::Range>,
}

/// The injections `query` finds in `root`'s tree, in document order.
/// `parent_ranges` (the parent layer's included ranges, if it is not the
/// root) bound every range. `None` once `limit` is reached.
fn find_injections(
    query: &InjectionQuery,
    root: Node,
    snapshot: &BufferSnapshot,
    parent_ranges: Option<&[tree_sitter::Range]>,
    limit: Limit,
) -> Option<Vec<Injection>> {
    let len = snapshot.len_bytes();
    let text = |node: Node| {
        let range = node.byte_range();
        snapshot
            .chunks_in_range(range.start.min(len)..range.end.min(len))
            .map(str::as_bytes)
    };
    let mut out: Vec<Injection> = Vec::new();
    // Combined injections: (pattern, language) to their index in `out`.
    let mut combined: HashMap<(usize, String), usize> = HashMap::new();
    let cancelled = Cell::new(false);
    let mut progress = |_: &QueryCursorState| {
        if limit.reached() {
            cancelled.set(true);
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = if limit.is_bounded() {
        QueryCursorOptions::new().progress_callback(&mut progress)
    } else {
        QueryCursorOptions::new()
    };
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches_with_options(&query.query, root, text, options);
    while let Some(m) = matches.next() {
        // The callback runs only now and then; small trees never reach it.
        if limit.reached() {
            return None;
        }
        if !query.predicates.accepts(m) {
            continue;
        }
        let settings = &query.patterns[m.pattern_index];
        let from_text = m
            .captures()
            .iter()
            .find(|c| query.language.contains(&c.index))
            .map(|c| {
                let range = c.node.byte_range();
                let start = range.start.min(len);
                let mut end = range.end.min(start + MAX_NAME_LEN).min(len);
                // Never cut a multi-byte char.
                while end > start && end < len && snapshot.byte_at(end) & 0xC0 == 0x80 {
                    end -= 1;
                }
                snapshot.text_for_range(start..end).trim().to_string()
            });
        let Some(language) = from_text.or_else(|| settings.language.as_deref().map(String::from))
        else {
            continue;
        };
        if language.is_empty() {
            continue;
        }
        let nodes = m
            .captures()
            .iter()
            .filter(|c| query.content.contains(&c.index))
            .map(|c| c.node.range());
        if settings.combined {
            let index = *combined
                .entry((m.pattern_index, language.clone()))
                .or_insert_with(|| {
                    out.push(Injection {
                        language,
                        ranges: Vec::new(),
                    });
                    out.len() - 1
                });
            out[index].ranges.extend(nodes);
        } else {
            out.push(Injection {
                language,
                ranges: nodes.collect(),
            });
        }
    }
    if cancelled.get() {
        return None;
    }
    for injection in &mut out {
        let ranges = normalize(std::mem::take(&mut injection.ranges));
        injection.ranges = match parent_ranges {
            Some(parent) => intersect(&ranges, parent),
            None => ranges,
        };
    }
    out.retain(|i| !i.ranges.is_empty());
    Some(out)
}

/// Sorted, without empty ranges or ranges nested in an earlier one (nodes
/// either nest or are disjoint).
fn normalize(mut ranges: Vec<tree_sitter::Range>) -> Vec<tree_sitter::Range> {
    ranges.sort_by_key(|r| (r.start_byte, std::cmp::Reverse(r.end_byte)));
    let mut out: Vec<tree_sitter::Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.start_byte >= range.end_byte {
            continue;
        }
        if out
            .last()
            .is_some_and(|last| range.start_byte < last.end_byte)
        {
            continue;
        }
        out.push(range);
    }
    out
}

/// The parts of `ranges` inside `parent` (both sorted and disjoint).
fn intersect(
    ranges: &[tree_sitter::Range],
    parent: &[tree_sitter::Range],
) -> Vec<tree_sitter::Range> {
    let mut out = Vec::new();
    for range in ranges {
        let first = parent.partition_point(|p| p.end_byte <= range.start_byte);
        for p in parent[first..]
            .iter()
            .take_while(|p| p.start_byte < range.end_byte)
        {
            let (start_byte, start_point) = if p.start_byte > range.start_byte {
                (p.start_byte, p.start_point)
            } else {
                (range.start_byte, range.start_point)
            };
            let (end_byte, end_point) = if p.end_byte < range.end_byte {
                (p.end_byte, p.end_point)
            } else {
                (range.end_byte, range.end_point)
            };
            if start_byte < end_byte {
                out.push(tree_sitter::Range {
                    start_byte,
                    end_byte,
                    start_point,
                    end_point,
                });
            }
        }
    }
    out
}

/// An injected region parsed as `language`.
#[derive(Clone)]
pub(crate) struct Layer {
    pub language: Arc<Language>,
    /// 1 for a child of the root.
    pub depth: usize,
    pub ranges: Vec<tree_sitter::Range>,
    pub tree: Tree,
}

impl Layer {
    /// Whether the layer has text in `bytes`.
    fn intersects(&self, bytes: &Range<usize>) -> bool {
        let first = self.ranges.partition_point(|r| r.end_byte <= bytes.start);
        self.ranges
            .get(first)
            .is_some_and(|r| r.start_byte < bytes.end)
    }

    fn start(&self) -> usize {
        self.ranges.first().map_or(0, |r| r.start_byte)
    }

    fn end(&self) -> usize {
        self.ranges.last().map_or(0, |r| r.end_byte)
    }
}

/// The child layers of a document, sorted by start so a viewport finds its
/// layers without visiting all of them (Markdown's inline layers are not
/// capped, so there can be many).
#[derive(Default, Clone)]
pub(crate) struct Layers {
    list: Vec<Layer>,
    /// `max_end[i]`: the largest end among `list[..=i]`.
    max_end: Vec<usize>,
}

impl Layers {
    fn new(mut list: Vec<Layer>) -> Layers {
        list.sort_by_key(Layer::start);
        let max_end = list
            .iter()
            .scan(0, |max, layer| {
                *max = layer.end().max(*max);
                Some(*max)
            })
            .collect();
        Layers { list, max_end }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        self.list.iter()
    }

    /// The layers edited by `edits` (as from `input_edits`: last edit
    /// first, in the old text's coordinates): each tree is edited and each
    /// range moves with the text, as the root tree does. A range end
    /// inside or touching an edit stretches over the new text, and so does
    /// a start inside one or touching an insertion, so text typed into a
    /// region keeps its colours until the next parse. A layer gets the
    /// edits reaching into it one by one, and those before it merged into
    /// one (they only shift it); those after it leave it alone. A layer
    /// left with no text is dropped. Past `deadline`, every layer is
    /// dropped: the next parse rebuilds them.
    pub fn edited(&self, edits: &[InputEdit], deadline: Instant) -> Layers {
        // In text order, to search.
        let asc: Vec<&InputEdit> = edits.iter().rev().collect();
        let Some(first) = asc.first().map(|e| e.start_byte) else {
            return self.clone();
        };
        // `merged[i]`: the first `i + 1` edits as one.
        let mut merged: Vec<InputEdit> = Vec::with_capacity(asc.len());
        for &edit in &asc {
            let next = match merged.last() {
                None => *edit,
                Some(before) => {
                    let (new_end_byte, new_end_position) =
                        shift(before, edit.new_end_byte, edit.new_end_position);
                    InputEdit {
                        start_byte: before.start_byte,
                        start_position: before.start_position,
                        old_end_byte: edit.old_end_byte,
                        old_end_position: edit.old_end_position,
                        new_end_byte,
                        new_end_position,
                    }
                }
            };
            merged.push(next);
        }
        let mut list = Vec::with_capacity(self.list.len());
        for (i, layer) in self.list.iter().enumerate() {
            if i % 64 == 63 && Instant::now() > deadline {
                return Layers::default();
            }
            let mut layer = layer.clone();
            if layer.end() < first {
                list.push(layer);
                continue;
            }
            let (start, end) = (layer.start(), layer.end());
            let before = asc.partition_point(|e| e.old_end_byte < start);
            let reaching = asc.partition_point(|e| e.start_byte <= end);
            let into = asc[before..reaching].iter().rev().copied();
            for edit in into.chain(before.checked_sub(1).map(|i| &merged[i])) {
                layer.tree.edit(edit);
                for range in &mut layer.ranges {
                    edit_range(range, edit);
                }
            }
            layer.ranges.retain(|r| r.start_byte < r.end_byte);
            if !layer.ranges.is_empty() {
                list.push(layer);
            }
        }
        Layers::new(list)
    }

    /// The tree of the layer of `language` at `depth` whose text starts at
    /// `start`: the old tree to reparse that region from.
    fn tree_at(&self, language: &Arc<Language>, depth: usize, start: usize) -> Option<&Tree> {
        let from = self.list.partition_point(|l| l.start() < start);
        self.list[from..]
            .iter()
            .take_while(|l| l.start() == start)
            .find(|l| l.depth == depth && Arc::ptr_eq(&l.language, language))
            .map(|l| &l.tree)
    }

    /// The layers with text in `bytes`.
    pub fn intersecting(&self, bytes: &Range<usize>) -> impl Iterator<Item = &Layer> {
        let before = self.list.partition_point(|l| l.start() < bytes.end);
        (0..before)
            .rev()
            .take_while(|&i| self.max_end[i] > bytes.start)
            .map(|i| &self.list[i])
            .filter(|l| l.intersects(bytes))
    }
}

/// Where `byte` / `point`, at or after `edit`'s old end, is once `edit` is
/// applied.
fn shift(edit: &InputEdit, byte: usize, point: tree_sitter::Point) -> (usize, tree_sitter::Point) {
    let point = if point.row == edit.old_end_position.row {
        tree_sitter::Point::new(
            edit.new_end_position.row,
            edit.new_end_position.column + point.column - edit.old_end_position.column,
        )
    } else {
        tree_sitter::Point::new(
            point.row - edit.old_end_position.row + edit.new_end_position.row,
            point.column,
        )
    };
    (byte - edit.old_end_byte + edit.new_end_byte, point)
}

/// Moves `range` with the text through `edit` (see [`Layers::edited`]).
fn edit_range(range: &mut tree_sitter::Range, edit: &InputEdit) {
    // The end: after (or touching) the edit, it shifts; inside, it stretches
    // to the end of the new text.
    if range.end_byte >= edit.old_end_byte {
        (range.end_byte, range.end_point) = shift(edit, range.end_byte, range.end_point);
    } else if range.end_byte > edit.start_byte {
        (range.end_byte, range.end_point) = (edit.new_end_byte, edit.new_end_position);
    }
    // The start: at or before the edit's start it stays; inside, it moves
    // to the start of the new text; after, it shifts.
    if range.start_byte > edit.start_byte {
        if range.start_byte >= edit.old_end_byte {
            (range.start_byte, range.start_point) =
                shift(edit, range.start_byte, range.start_point);
        } else {
            (range.start_byte, range.start_point) = (edit.start_byte, edit.start_position);
        }
    }
}

/// Markdown's inline text, injected by the bundled Markdown into the
/// bundled `markdown_inline`, is part of Markdown itself (Phase 1 drew it
/// without layers): those layers do not count toward [`MAX_LAYERS`], so a
/// long file keeps its inline colours.
fn uncapped(parent: &Language, child: &Language) -> bool {
    is_bundled(parent, "markdown") && is_bundled(child, "markdown_inline")
}

fn is_bundled(language: &Language, id: &str) -> bool {
    *language.origin() == LanguageOrigin::Bundled && &*language.id == id
}

/// One parser per injected language, kept across parses. Layers keep only
/// their trees: a parser per layer (up to 1000, each possibly holding a
/// WASM store) would cost far more than it saves; an incremental parse
/// reuses a layer's old tree through the shared parser instead.
#[derive(Default)]
pub(crate) struct LayerParsers {
    parsers: Vec<(Arc<Language>, Parser)>,
}

impl Drop for LayerParsers {
    fn drop(&mut self) {
        for (_, parser) in &mut self.parsers {
            if let Some(store) = parser.take_wasm_store() {
                wasm::return_store(store);
            }
        }
    }
}

impl LayerParsers {
    /// The parser for `language`, made on first use. `None` when its grammar
    /// cannot be loaded.
    fn get(&mut self, language: &Arc<Language>) -> Option<&mut Parser> {
        if let Some(i) = self
            .parsers
            .iter()
            .position(|(l, _)| Arc::ptr_eq(l, language))
        {
            return Some(&mut self.parsers[i].1);
        }
        let grammar = language.grammar().ok()?;
        let mut parser = Parser::new();
        if language.is_wasm() {
            parser.set_wasm_store(wasm::take_store().ok()?).ok()?;
        }
        if let Err(e) = parser.set_language(&grammar) {
            tracing::warn!("grammar for {} is incompatible: {e}", language.id);
            language.mark_failed();
            if let Some(store) = parser.take_wasm_store() {
                wasm::return_store(store);
            }
            return None;
        }
        self.parsers.push((language.clone(), parser));
        self.parsers.last_mut().map(|(_, p)| p)
    }
}

/// Builds the child layers of `root_tree` (parsed as `root_language`),
/// breadth first, up to [`MAX_DEPTH`] and [`MAX_LAYERS`].
///
/// `limit` (a deadline, a cancel flag) bounds the whole build: it is
/// checked before each injections query (and while it runs) and before
/// each child parse. Once reached, the layers built so far are kept and the
/// result is `TimedOut`.
///
/// With a deadline (the synchronous attempt on the UI thread), a WASM
/// grammar not loaded yet is not loaded here: its region is skipped and the
/// result is `TimedOut`, so the caller finishes the parse in the background,
/// where the grammar loads.
///
/// `max_children` (tests only) stops the build the same way once that many
/// layers are built, so the partial outcome can be tested without timing.
///
/// `old` (an incremental parse) are layers whose trees describe `snapshot`
/// once edited (see [`Layers::edited`]): a region found again at the same
/// start, in the same language and depth, is reparsed from its old tree.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_layers(
    root_language: &Arc<Language>,
    root_tree: &Tree,
    snapshot: &BufferSnapshot,
    registry: &LanguageRegistry,
    parsers: &mut LayerParsers,
    limit: Limit,
    max_children: Option<usize>,
    old: Option<&Layers>,
) -> (Layers, ParseOutcome) {
    let mut layers: Vec<Layer> = Vec::new();
    let outcome = build(
        root_language,
        root_tree,
        snapshot,
        registry,
        parsers,
        limit,
        max_children,
        old,
        &mut layers,
    );
    (Layers::new(layers), outcome)
}

#[allow(clippy::too_many_arguments)]
fn build(
    root_language: &Arc<Language>,
    root_tree: &Tree,
    snapshot: &BufferSnapshot,
    registry: &LanguageRegistry,
    parsers: &mut LayerParsers,
    limit: Limit,
    max_children: Option<usize>,
    old: Option<&Layers>,
    layers: &mut Vec<Layer>,
) -> ParseOutcome {
    let out_of_budget =
        |built: usize| limit.reached() || max_children.is_some_and(|max| built >= max);
    let mut outcome = ParseOutcome::Done;
    // Layers that count toward the cap.
    let mut counted = 0;
    let mut reported = false;
    // 0 is the root; `i > 0` is `layers[i - 1]`.
    for parent in 0.. {
        let (parent_language, tree, depth, ranges) = match parent {
            0 => (root_language, root_tree, 0, None),
            i => {
                let Some(layer) = layers.get(i - 1) else {
                    break;
                };
                (
                    &layer.language,
                    &layer.tree,
                    layer.depth,
                    Some(layer.ranges.as_slice()),
                )
            }
        };
        if depth >= MAX_DEPTH {
            continue;
        }
        let Some(query) = parent_language
            .queries()
            .and_then(|q| q.injections.as_ref())
        else {
            continue;
        };
        // Past the cap, only Markdown can still add (uncapped) layers.
        let may_be_uncapped = is_bundled(parent_language, "markdown");
        if counted >= MAX_LAYERS && !may_be_uncapped {
            continue;
        }
        if out_of_budget(layers.len()) {
            return ParseOutcome::TimedOut;
        }
        let Some(injections) = find_injections(query, tree.root_node(), snapshot, ranges, limit)
        else {
            return ParseOutcome::TimedOut;
        };
        let parent_language = parent_language.clone();
        for injection in injections {
            let Some(language) = registry.language_by_name(&injection.language) else {
                continue;
            };
            let exempt = uncapped(&parent_language, &language);
            if !exempt && counted >= MAX_LAYERS {
                if !reported {
                    reported = true;
                    tracing::debug!(
                        "more than {MAX_LAYERS} injected layers; the rest are not highlighted"
                    );
                }
                continue;
            }
            if language.is_failed() {
                continue;
            }
            if limit.deadline.is_some() && !language.grammar_ready() {
                outcome = ParseOutcome::TimedOut;
                continue;
            }
            if language.queries().is_none() {
                continue;
            }
            let Some(parser) = parsers.get(&language) else {
                continue;
            };
            if parser.set_included_ranges(&injection.ranges).is_err() {
                continue;
            }
            // Small parses never reach tree-sitter's progress callback.
            if out_of_budget(layers.len()) {
                return ParseOutcome::TimedOut;
            }
            let start = injection.ranges.first().map_or(0, |r| r.start_byte);
            let seed = old.and_then(|old| old.tree_at(&language, depth + 1, start));
            let Some(tree) = parse_snapshot(parser, snapshot, seed, limit) else {
                return ParseOutcome::TimedOut;
            };
            if !exempt {
                counted += 1;
            }
            layers.push(Layer {
                language,
                depth: depth + 1,
                ranges: injection.ranges,
                tree,
            });
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Point;

    fn r(start: usize, end: usize) -> tree_sitter::Range {
        tree_sitter::Range {
            start_byte: start,
            end_byte: end,
            start_point: Point::new(0, start),
            end_point: Point::new(0, end),
        }
    }

    fn bytes(ranges: &[tree_sitter::Range]) -> Vec<(usize, usize)> {
        ranges.iter().map(|r| (r.start_byte, r.end_byte)).collect()
    }

    #[test]
    fn normalize_sorts_and_drops_nested_and_empty() {
        let out = normalize(vec![r(10, 20), r(0, 5), r(12, 15), r(7, 7)]);
        assert_eq!(bytes(&out), vec![(0, 5), (10, 20)]);
    }

    #[test]
    fn intersect_clips_to_parent_ranges() {
        let parent = [r(0, 4), r(6, 10), r(20, 30)];
        let out = intersect(&[r(2, 8), r(9, 25), r(11, 19)], &parent);
        assert_eq!(bytes(&out), vec![(2, 4), (6, 8), (9, 10), (20, 25)]);
        assert_eq!(out[0].start_point, Point::new(0, 2));
        assert_eq!(out[1].start_point, Point::new(0, 6));
    }
}
