//! `outline.scm`: symbols from a tree-sitter query (spec §3.2), and the
//! shared symbol tree.

use std::ops::Range;
use std::sync::Arc;

use lucerna_syntax::{
    Language, LanguageRegistry, ParseOutcome, Symbol, SymbolKind, SymbolTree, SyntaxState,
};
use lucerna_text::{Buffer, BufferSnapshot, Point};

/// Written for these tests: functions and impl blocks of Rust.
const OUTLINE: &str = r#"
(function_item "fn" @context name: (_) @name) @item
(impl_item "impl" @context type: (_) @name) @item
"#;

const SOURCE: &str = "fn top() {}\n\nimpl Foo {\n    fn bar(&self) {}\n    fn baz() {}\n}\n";

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn parsed(language: Arc<Language>, text: &str) -> (SyntaxState, BufferSnapshot) {
    let snap = snapshot(text);
    let mut state = SyntaxState::new(language, Arc::new(LanguageRegistry::bundled())).unwrap();
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    (state, snap)
}

fn symbol(name: &str, kind: SymbolKind, range: Range<Point>) -> Symbol {
    Symbol {
        name: name.into(),
        detail: None,
        kind,
        selection_range: range.clone(),
        range,
        children: Vec::new(),
    }
}

#[test]
fn outline_nests_by_range_and_guesses_kind() {
    let rust = Language::bundled_with_outline_for_test("rust", OUTLINE);
    assert!(rust.has_outline());
    let (state, snap) = parsed(rust, SOURCE);
    let tree = state
        .outline(&snap)
        .expect("the language has an outline query");

    let flat: Vec<(usize, Vec<&str>, &str, SymbolKind)> = tree
        .flatten()
        .into_iter()
        .map(|f| (f.depth, f.ancestors, f.symbol.name.as_str(), f.symbol.kind))
        .collect();
    assert_eq!(
        flat,
        vec![
            (0, vec![], "top", SymbolKind::Function),
            (0, vec![], "Foo", SymbolKind::Object),
            (1, vec!["Foo"], "bar", SymbolKind::Function),
            (1, vec!["Foo"], "baz", SymbolKind::Function),
        ]
    );

    let top = &tree.0[0];
    assert_eq!(top.detail.as_deref(), Some("fn"));
    assert_eq!(top.range, pt(0, 0)..pt(0, 11));
    assert_eq!(top.selection_range, pt(0, 3)..pt(0, 6));
    let foo = &tree.0[1];
    assert_eq!(foo.detail.as_deref(), Some("impl"));
    assert_eq!(foo.range, pt(2, 0)..pt(5, 1));
    assert_eq!(foo.selection_range, pt(2, 5)..pt(2, 8));
    assert_eq!(foo.children[0].selection_range, pt(3, 7)..pt(3, 10));

    let path: Vec<&str> = tree
        .path_at(pt(3, 12))
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(path, vec!["Foo", "bar"]);
    assert!(tree.path_at(pt(1, 0)).is_empty());
}

#[test]
fn outline_needs_a_query() {
    let rust = LanguageRegistry::bundled()
        .language_by_name("rust")
        .unwrap();
    assert!(!rust.has_outline());
    let (state, snap) = parsed(rust, SOURCE);
    assert!(state.outline(&snap).is_none());
}

#[test]
fn from_flat_nests_by_containment_in_document_order() {
    let tree = SymbolTree::from_flat(vec![
        symbol("inner", SymbolKind::Method, pt(1, 0)..pt(2, 0)),
        symbol("after", SymbolKind::Function, pt(5, 0)..pt(6, 0)),
        symbol("outer", SymbolKind::Class, pt(0, 0)..pt(4, 0)),
        symbol("deep", SymbolKind::Variable, pt(1, 4)..pt(1, 8)),
    ]);
    let flat: Vec<(usize, &str)> = tree
        .flatten()
        .into_iter()
        .map(|f| (f.depth, f.symbol.name.as_str()))
        .collect();
    assert_eq!(
        flat,
        vec![(0, "outer"), (1, "inner"), (2, "deep"), (0, "after")]
    );
    assert_eq!(tree.flatten()[2].ancestors, vec!["outer", "inner"]);
}
