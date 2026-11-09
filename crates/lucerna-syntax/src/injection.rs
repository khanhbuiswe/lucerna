//! Injections: reading an `injections.scm` query, and the tree of layers
//! (one tree per injected region) that [`crate::SyntaxState`] keeps under
//! its root tree.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use lucerna_text::BufferSnapshot;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

use crate::registry::{Language, LanguageRegistry};
use crate::syntax_state::{ParseOutcome, parse_snapshot};
use crate::wasm;

/// The deepest a layer may be; the root is depth 0.
pub(crate) const MAX_DEPTH: usize = 4;

/// Most child layers in one document.
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
    pub(crate) fn new(query: Query) -> InjectionQuery {
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
        InjectionQuery {
            query,
            content,
            language,
            patterns,
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
/// root) bound every range.
fn find_injections(
    query: &InjectionQuery,
    root: Node,
    snapshot: &BufferSnapshot,
    parent_ranges: Option<&[tree_sitter::Range]>,
) -> Vec<Injection> {
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
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query.query, root, text);
    while let Some(m) = matches.next() {
        let settings = &query.patterns[m.pattern_index];
        let from_text = m
            .captures()
            .iter()
            .find(|c| query.language.contains(&c.index))
            .map(|c| {
                let range = c.node.byte_range();
                let end = range.end.min(range.start + MAX_NAME_LEN).min(len);
                snapshot
                    .text_for_range(range.start.min(end)..end)
                    .trim()
                    .to_string()
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
    for injection in &mut out {
        let ranges = normalize(std::mem::take(&mut injection.ranges));
        injection.ranges = match parent_ranges {
            Some(parent) => intersect(&ranges, parent),
            None => ranges,
        };
    }
    out.retain(|i| !i.ranges.is_empty());
    out
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
pub(crate) struct Layer {
    pub language: Arc<Language>,
    /// 1 for a child of the root.
    pub depth: usize,
    pub ranges: Vec<tree_sitter::Range>,
    pub tree: Tree,
}

impl Layer {
    /// Whether the layer has text in `bytes`.
    pub fn intersects(&self, bytes: &Range<usize>) -> bool {
        let first = self.ranges.partition_point(|r| r.end_byte <= bytes.start);
        self.ranges
            .get(first)
            .is_some_and(|r| r.start_byte < bytes.end)
    }
}

/// One parser per injected language, kept across parses. Layers keep only
/// their trees: a parser per layer (up to 1000, each possibly holding a
/// WASM store) would cost far more than the reparse it could speed up, as
/// layers are rebuilt from scratch anyway.
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

/// Builds the child layers of `root_tree` (parsed as `root_language`) into
/// `layers`, breadth first, up to [`MAX_DEPTH`] and [`MAX_LAYERS`].
///
/// With a `deadline` (the synchronous attempt on the UI thread), a WASM
/// grammar not loaded yet is not loaded here: its region is skipped and the
/// result is `TimedOut`, so the caller finishes the parse in the background,
/// where the grammar loads. A child parse past the deadline keeps the
/// layers built so far and returns `TimedOut` too.
pub(crate) fn build_layers(
    root_language: &Arc<Language>,
    root_tree: &Tree,
    snapshot: &BufferSnapshot,
    registry: &LanguageRegistry,
    parsers: &mut LayerParsers,
    deadline: Option<Instant>,
    layers: &mut Vec<Layer>,
) -> ParseOutcome {
    let mut outcome = ParseOutcome::Done;
    // `None` is the root; `Some(i)` is `layers[i]`.
    let mut parent: Option<usize> = None;
    loop {
        let (language, tree, depth, ranges) = match parent {
            None => (root_language, root_tree, 0, None),
            Some(i) => {
                let Some(layer) = layers.get(i) else {
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
        let injections = match language.queries().and_then(|q| q.injections.as_ref()) {
            Some(query) if depth < MAX_DEPTH => {
                find_injections(query, tree.root_node(), snapshot, ranges)
            }
            _ => Vec::new(),
        };
        let child_depth = depth + 1;
        for injection in injections {
            if layers.len() >= MAX_LAYERS {
                tracing::debug!(
                    "more than {MAX_LAYERS} injected layers; the rest are not highlighted"
                );
                return outcome;
            }
            let Some(language) = registry.language_by_name(&injection.language) else {
                continue;
            };
            if language.is_failed() {
                continue;
            }
            if deadline.is_some() && !language.grammar_ready() {
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
            match parse_snapshot(parser, snapshot, deadline) {
                Some(tree) => layers.push(Layer {
                    language,
                    depth: child_depth,
                    ranges: injection.ranges,
                    tree,
                }),
                None => return ParseOutcome::TimedOut,
            }
        }
        parent = Some(parent.map_or(0, |i| i + 1));
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
