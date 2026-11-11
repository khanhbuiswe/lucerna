//! `textDocument/documentSymbol` answers as a [`SymbolTree`] (spec §3.1).

use lucerna_syntax::{Symbol, SymbolKind, SymbolTree};
use lucerna_text::BufferSnapshot;

use crate::position::{PositionEncoding, range_from_lsp};

/// The symbols of a `documentSymbol` answer, with positions in `snapshot`
/// (the text the server was sent). A hierarchical answer
/// (`DocumentSymbol[]`) keeps its nesting, each level sorted by start; a
/// flat one (`SymbolInformation[]`) is nested by range containment, and
/// each symbol's selection range is its range.
pub fn symbols_from_response(
    resp: lsp_types::DocumentSymbolResponse,
    snapshot: &BufferSnapshot,
    enc: PositionEncoding,
) -> SymbolTree {
    match resp {
        lsp_types::DocumentSymbolResponse::Flat(infos) => SymbolTree::from_flat(
            infos
                .into_iter()
                .map(|info| {
                    let range = range_from_lsp(snapshot, info.location.range, enc);
                    Symbol {
                        name: info.name,
                        detail: None,
                        kind: symbol_kind(info.kind),
                        selection_range: range.clone(),
                        range,
                        children: Vec::new(),
                    }
                })
                .collect(),
        ),
        lsp_types::DocumentSymbolResponse::Nested(symbols) => {
            SymbolTree(nested(symbols, snapshot, enc))
        }
    }
}

fn nested(
    symbols: Vec<lsp_types::DocumentSymbol>,
    snapshot: &BufferSnapshot,
    enc: PositionEncoding,
) -> Vec<Symbol> {
    let mut out: Vec<Symbol> = symbols
        .into_iter()
        .map(|s| Symbol {
            name: s.name,
            detail: s.detail.filter(|d| !d.is_empty()),
            kind: symbol_kind(s.kind),
            range: range_from_lsp(snapshot, s.range, enc),
            selection_range: range_from_lsp(snapshot, s.selection_range, enc),
            children: nested(s.children.unwrap_or_default(), snapshot, enc),
        })
        .collect();
    out.sort_by_key(|s| (s.range.start.row, s.range.start.column));
    out
}

/// LSP's kinds in the order of [`SymbolKind::ALL`].
const LSP_KINDS: [lsp_types::SymbolKind; 26] = {
    use lsp_types::SymbolKind as K;
    [
        K::FILE,
        K::MODULE,
        K::NAMESPACE,
        K::PACKAGE,
        K::CLASS,
        K::METHOD,
        K::PROPERTY,
        K::FIELD,
        K::CONSTRUCTOR,
        K::ENUM,
        K::INTERFACE,
        K::FUNCTION,
        K::VARIABLE,
        K::CONSTANT,
        K::STRING,
        K::NUMBER,
        K::BOOLEAN,
        K::ARRAY,
        K::OBJECT,
        K::KEY,
        K::NULL,
        K::ENUM_MEMBER,
        K::STRUCT,
        K::EVENT,
        K::OPERATOR,
        K::TYPE_PARAMETER,
    ]
};

/// An LSP symbol kind as Lucerna's; unknown kinds are `Object`.
pub fn symbol_kind(kind: lsp_types::SymbolKind) -> SymbolKind {
    LSP_KINDS
        .iter()
        .position(|k| *k == kind)
        .map_or(SymbolKind::Object, |i| SymbolKind::ALL[i])
}
