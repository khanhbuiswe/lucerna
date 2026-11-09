//! Spike: tree-sitter's `wasm` feature loads a grammar built with wasi-sdk.

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator, WasmStore, wasmtime};

const JSON_WASM: &[u8] = include_bytes!("fixtures/wasm/json.wasm");
const SAMPLE: &str = include_str!("fixtures/sample.json");

fn load(engine: &wasmtime::Engine) -> (WasmStore, Language) {
    let mut store = WasmStore::new(engine).expect("wasm store");
    let lang = store
        .load_language("json", JSON_WASM)
        .expect("load json.wasm");
    (store, lang)
}

#[test]
fn wasm_json_loads_and_parses() {
    let engine = wasmtime::Engine::default();
    let (store, lang) = load(&engine);
    assert!(lang.is_wasm());
    let mut parser = Parser::new();
    parser.set_wasm_store(store).expect("set store");
    parser.set_language(&lang).expect("set language");
    let tree = parser.parse(r#"{"a": [1, true]}"#, None).expect("parse");
    assert_eq!(tree.root_node().kind(), "document");
    assert!(!tree.root_node().has_error());
}

#[test]
fn wasm_language_shared_across_stores() {
    let engine = wasmtime::Engine::default();
    let (_store_a, lang) = load(&engine);
    let store_b = WasmStore::new(&engine).expect("store b");
    let mut parser = Parser::new();
    parser.set_wasm_store(store_b).expect("set store b");
    parser
        .set_language(&lang)
        .expect("set language from store a");
    let tree = parser.parse(r#"{"a": [1, true]}"#, None).expect("parse");
    assert_eq!(tree.root_node().kind(), "document");
    assert!(!tree.root_node().has_error());
}

fn captures(lang: &Language, tree: &tree_sitter::Tree, src: &str) -> Vec<(usize, usize, String)> {
    let query = Query::new(lang, tree_sitter_json::HIGHLIGHTS_QUERY).expect("query");
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut it = cursor.captures(&query, tree.root_node(), src.as_bytes());
    while let Some((m, idx)) = it.next() {
        let c = m.captures()[*idx];
        out.push((
            c.node.start_byte(),
            c.node.end_byte(),
            names[c.index as usize].to_string(),
        ));
    }
    out
}

#[test]
fn wasm_matches_native_highlights() {
    let engine = wasmtime::Engine::default();
    let (store, wasm_lang) = load(&engine);
    let mut parser = Parser::new();
    parser.set_wasm_store(store).expect("set store");
    parser.set_language(&wasm_lang).expect("set language");
    let wasm_tree = parser.parse(SAMPLE, None).expect("wasm parse");
    let wasm_caps = captures(&wasm_lang, &wasm_tree, SAMPLE);

    let native_lang: Language = tree_sitter_json::LANGUAGE.into();
    let mut native = Parser::new();
    native.set_language(&native_lang).expect("native language");
    let native_tree = native.parse(SAMPLE, None).expect("native parse");
    let native_caps = captures(&native_lang, &native_tree, SAMPLE);

    assert!(!native_caps.is_empty());
    assert_eq!(wasm_caps, native_caps);
}

#[test]
fn wasm_load_garbage_errors() {
    let engine = wasmtime::Engine::default();
    let mut store = WasmStore::new(&engine).expect("store");
    assert!(store.load_language("json", b"not wasm").is_err());
}
