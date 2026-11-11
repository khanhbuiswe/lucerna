//! `textDocument/documentSymbol` answers as a `SymbolTree` (spec §3.1).

use lsp_types::DocumentSymbolResponse;
use lucerna_lsp::position::PositionEncoding;
use lucerna_lsp::symbols::symbols_from_response;
use lucerna_syntax::SymbolKind;
use lucerna_text::{Buffer, BufferSnapshot, Point};
use serde_json::{Value, json};

fn snap(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn range(l1: u32, c1: u32, l2: u32, c2: u32) -> Value {
    json!({ "start": { "line": l1, "character": c1 }, "end": { "line": l2, "character": c2 } })
}

fn response(value: Value) -> DocumentSymbolResponse {
    serde_json::from_value(value).unwrap()
}

const TEXT: &str = "struct A {\n    x: u32,\n}\nimpl A {\n    fn new() {}\n}\n";

#[test]
fn flat_symbol_information_nests() {
    let info = |name: &str, kind: u32, r: Value| {
        json!({ "name": name, "kind": kind,
                "location": { "uri": "file:///a.rs", "range": r } })
    };
    let resp = response(json!([
        info("new", 6, range(4, 4, 4, 15)),
        info("A", 23, range(0, 0, 2, 1)),
        info("x", 8, range(1, 4, 1, 10)),
        info("impl A", 19, range(3, 0, 5, 1)),
    ]));
    let tree = symbols_from_response(resp, &snap(TEXT), PositionEncoding::Utf16);
    let flat: Vec<(usize, &str, SymbolKind)> = tree
        .flatten()
        .into_iter()
        .map(|f| (f.depth, f.symbol.name.as_str(), f.symbol.kind))
        .collect();
    assert_eq!(
        flat,
        vec![
            (0, "A", SymbolKind::Struct),
            (1, "x", SymbolKind::Field),
            (0, "impl A", SymbolKind::Object),
            (1, "new", SymbolKind::Method),
        ]
    );
    // A flat symbol's selection range is its range.
    assert_eq!(tree.0[0].selection_range, pt(0, 0)..pt(2, 1));
}

#[test]
fn hierarchical_kept() {
    let resp = response(json!([
        { "name": "A", "detail": "struct", "kind": 23,
          "range": range(0, 0, 2, 1), "selectionRange": range(0, 7, 0, 8),
          "children": [
            { "name": "x", "kind": 8, "range": range(1, 4, 1, 10),
              "selectionRange": range(1, 4, 1, 5) }
          ] },
        { "name": "impl A", "kind": 19, "range": range(3, 0, 5, 1),
          "selectionRange": range(3, 5, 3, 6), "children": [] },
        { "name": "odd", "kind": 999, "range": range(4, 4, 4, 15),
          "selectionRange": range(4, 7, 4, 10) }
    ]));
    let tree = symbols_from_response(resp, &snap(TEXT), PositionEncoding::Utf16);
    assert_eq!(tree.0.len(), 3, "a hierarchical answer is not re-nested");
    let a = &tree.0[0];
    assert_eq!(a.detail.as_deref(), Some("struct"));
    assert_eq!(a.range, pt(0, 0)..pt(2, 1));
    assert_eq!(a.selection_range, pt(0, 7)..pt(0, 8));
    assert_eq!(a.children.len(), 1);
    assert_eq!(a.children[0].name, "x");
    assert_eq!(a.children[0].kind, SymbolKind::Field);
    assert_eq!(tree.0[2].kind, SymbolKind::Object, "unknown kinds");
}

#[test]
fn utf16_selection_range() {
    // `😀` is two UTF-16 units and four bytes; `é` one unit and two bytes.
    let text = "let s = \"😀é\"; fn café() {}\n";
    let resp = response(json!([
        { "name": "café", "kind": 12, "range": range(0, 15, 0, 27),
          "selectionRange": range(0, 18, 0, 22) }
    ]));
    let tree = symbols_from_response(resp, &snap(text), PositionEncoding::Utf16);
    let f = &tree.0[0];
    let start = text.find("fn").unwrap() as u32;
    let name = text.find("café").unwrap() as u32;
    assert_eq!(
        f.range,
        pt(0, start)..pt(0, text.find('}').unwrap() as u32 + 1)
    );
    assert_eq!(
        f.selection_range,
        pt(0, name)..pt(0, name + "café".len() as u32)
    );
}
