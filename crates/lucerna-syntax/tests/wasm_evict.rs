//! Dropping cached WASM grammars no registry uses (alone in its binary:
//! the grammar cache is process-wide).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use lucerna_syntax::{
    Associations, ExtensionLanguage, LanguageRegistry, SyntaxState, retain_grammars, wasm_cache_len,
};
use lucerna_text::Buffer;

#[test]
fn retain_grammars_drops_unused_and_live_parsers_keep_working() {
    let dir = std::env::temp_dir().join(format!("lucerna-evict-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("highlights.scm"), "(string) @string\n").unwrap();
    let spec = ExtensionLanguage {
        extension_id: "evict".into(),
        name: "Evict".into(),
        dir: dir.clone(),
        grammar: Some((
            "json".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm/json.wasm"),
        )),
        path_suffixes: vec!["ev".into()],
        first_line_pattern: None,
        hidden: false,
    };
    let registry = Arc::new(LanguageRegistry::build(&[spec], Associations::default()));
    let language = registry.language_by_name("Evict").unwrap();
    let mut state = SyntaxState::new(language.clone(), registry.clone()).unwrap();
    let snap = Buffer::from_bytes(br#""a""#, false).unwrap().snapshot();
    state.parse(&snap, None);
    assert_eq!(wasm_cache_len(), 1);

    // Still used: kept.
    retain_grammars(&registry.grammar_keys());
    assert_eq!(wasm_cache_len(), 1);
    assert!(language.grammar_ready());

    // Uninstalled: dropped, but the open parser still parses.
    retain_grammars(&HashSet::new());
    assert_eq!(wasm_cache_len(), 0);
    let snap = Buffer::from_bytes(br#"["b", "c"]"#, false)
        .unwrap()
        .snapshot();
    state.parse(&snap, None);
    let spans = state.highlights(&snap, 0..1, &|_| true);
    assert_eq!(
        spans.iter().filter(|s| &*s.capture == "string").count(),
        2,
        "{spans:?}"
    );
    drop(state);
    let _ = std::fs::remove_dir_all(&dir);
}
