//! Symbols from an extension's `outline.scm` (spec §3.2): each `@item`
//! match is a symbol, `@name` its name, `@context` (less the name) its
//! detail and the source of its guessed kind. Nesting follows ranges.

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::registry::Predicates;
use crate::symbols::{Symbol, SymbolKind, SymbolTree};

/// A compiled `outline.scm` with its capture indices.
pub(crate) struct OutlineQuery {
    query: Query,
    item: u32,
    name: u32,
    context: Option<u32>,
    predicates: Predicates,
}

impl OutlineQuery {
    /// `None` (with a warning) when the query lacks `@item` or `@name`.
    pub(crate) fn new(query: Query, language_id: &str) -> Option<OutlineQuery> {
        let (Some(item), Some(name)) = (
            query.capture_index_for_name("item"),
            query.capture_index_for_name("name"),
        ) else {
            tracing::warn!("language {language_id}: outline query needs @item and @name");
            return None;
        };
        let context = query.capture_index_for_name("context");
        let predicates = Predicates::new(&query, language_id);
        Some(OutlineQuery {
            query,
            item,
            name,
            context,
            predicates,
        })
    }

    /// The symbols of `tree`, the parse of `snapshot`. Runs the query
    /// over the whole tree: call it off the UI thread for large files.
    pub(crate) fn symbols(&self, tree: &Tree, snapshot: &BufferSnapshot) -> SymbolTree {
        let len = snapshot.len_bytes();
        let text_of = |node: Node| {
            let range = node.byte_range();
            snapshot
                .chunks_in_range(range.start.min(len)..range.end.min(len))
                .map(str::as_bytes)
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), text_of);
        let mut items: Vec<Symbol> = Vec::new();
        while let Some(m) = matches.next() {
            if !self.predicates.accepts(m) {
                continue;
            }
            let Some(item) = m.nodes_for_capture_index(self.item).next() else {
                continue;
            };
            let names: Vec<Node> = m.nodes_for_capture_index(self.name).collect();
            let (Some(first), Some(last)) = (names.first(), names.last()) else {
                continue;
            };
            let name = words(
                &names
                    .iter()
                    .map(|n| text(snapshot, n.byte_range()))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if name.is_empty() {
                continue;
            }
            let contexts: Vec<String> = self
                .context
                .map(|c| {
                    m.nodes_for_capture_index(c)
                        .map(|node| context_text(snapshot, node, &names))
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let symbol = Symbol {
                name,
                detail: (!contexts.is_empty()).then(|| contexts.join(" ")),
                kind: guess_kind(&contexts),
                range: point(item.start_position())..point(item.end_position()),
                selection_range: point(first.start_position())..point(last.end_position()),
                children: Vec::new(),
            };
            // Several patterns can match the same node.
            if !items
                .iter()
                .any(|s| s.range == symbol.range && s.selection_range == symbol.selection_range)
            {
                items.push(symbol);
            }
        }
        SymbolTree::from_flat(items)
    }
}

fn point(p: tree_sitter::Point) -> Point {
    Point {
        row: p.row as u32,
        column: p.column as u32,
    }
}

/// The text of `range`, clamped to the snapshot.
fn text(snapshot: &BufferSnapshot, range: Range<usize>) -> String {
    let len = snapshot.len_bytes();
    snapshot
        .chunks_in_range(range.start.min(len)..range.end.min(len))
        .collect()
}

/// `text` with runs of whitespace (line breaks included) as one space.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of a `@context` node without the parts that are `@name`
/// nodes; empty when the name covers it.
fn context_text(snapshot: &BufferSnapshot, node: Node, names: &[Node]) -> String {
    let mut out = String::new();
    let mut at = node.start_byte();
    let end = node.end_byte();
    for name in names {
        let (start, stop) = (name.start_byte().max(at), name.end_byte().min(end));
        if start >= stop {
            continue;
        }
        out.push_str(&text(snapshot, at..start));
        out.push(' ');
        at = stop;
    }
    if at < end {
        out.push_str(&text(snapshot, at..end));
    }
    words(&out)
}

/// The kind named by the first keyword of the contexts that names one:
/// `fn` / `def` / `func` / `function`, `struct`, `class`, `enum`, `trait`
/// / `interface`, `impl`, `mod` / `module`, `namespace`, `const`, `type`.
/// `Object` otherwise.
pub(crate) fn guess_kind(contexts: &[String]) -> SymbolKind {
    contexts
        .iter()
        .flat_map(|c| c.split(|ch: char| !ch.is_alphanumeric() && ch != '_'))
        .find_map(|word| {
            Some(match word {
                "fn" | "def" | "func" | "function" => SymbolKind::Function,
                "struct" => SymbolKind::Struct,
                "class" => SymbolKind::Class,
                "enum" => SymbolKind::Enum,
                "trait" | "interface" => SymbolKind::Interface,
                // As rust-analyzer reports impl blocks.
                "impl" => SymbolKind::Object,
                "mod" | "module" => SymbolKind::Module,
                "namespace" => SymbolKind::Namespace,
                "const" => SymbolKind::Constant,
                // As rust-analyzer reports type aliases.
                "type" => SymbolKind::TypeParameter,
                _ => return None,
            })
        })
        .unwrap_or(SymbolKind::Object)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(contexts: &[&str]) -> SymbolKind {
        guess_kind(&contexts.iter().map(|c| c.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn kind_from_first_keyword() {
        assert_eq!(kind(&["def"]), SymbolKind::Function);
        assert_eq!(kind(&["pub(crate)", "fn"]), SymbolKind::Function);
        assert_eq!(kind(&["export class"]), SymbolKind::Class);
        assert_eq!(kind(&["struct"]), SymbolKind::Struct);
        assert_eq!(kind(&["trait"]), SymbolKind::Interface);
        assert_eq!(kind(&["interface"]), SymbolKind::Interface);
        assert_eq!(kind(&["enum"]), SymbolKind::Enum);
        assert_eq!(kind(&["mod"]), SymbolKind::Module);
        assert_eq!(kind(&["namespace"]), SymbolKind::Namespace);
        assert_eq!(kind(&["const"]), SymbolKind::Constant);
        assert_eq!(kind(&["type"]), SymbolKind::TypeParameter);
        assert_eq!(kind(&["impl"]), SymbolKind::Object);
        assert_eq!(kind(&["let"]), SymbolKind::Object);
        assert_eq!(kind(&[]), SymbolKind::Object);
    }
}
