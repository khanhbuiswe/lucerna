use std::path::PathBuf;
use std::sync::Arc;

use lucerna_syntax::{
    Language, LanguageRegistry, ParseOutcome, SyntaxState, validate_query, validate_wasm_grammar,
};
use lucerna_text::Buffer;

fn json_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm/json.wasm")
}

fn highlight(
    language: Arc<Language>,
    text: &str,
    styled: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
    let snap = buffer.snapshot();
    let mut state = SyntaxState::new(language, bundled()).expect("grammar loads");
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    state
        .highlights(&snap, 0..snap.line_count(), styled)
        .into_iter()
        .map(|s| (text[s.range].to_string(), s.capture.to_string()))
        .collect()
}

#[test]
fn extension_language_from_wasm_highlights_json() {
    let language =
        Language::from_wasm_for_test("t-json", &json_wasm(), "(string) @string\n(number) @number");
    let spans = highlight(language, r#"{"a": 1}"#, &|_| true);
    assert!(
        spans
            .iter()
            .any(|(t, c)| t == "\"a\"" && c.starts_with("string")),
        "{spans:?}"
    );
}

#[test]
fn missing_wasm_file_is_grammar_error() {
    let language =
        Language::from_wasm_for_test("t-missing", &PathBuf::from("/nope/x.wasm"), "(string) @s");
    assert!(!language.is_failed());
    assert!(SyntaxState::new(language.clone(), bundled()).is_err());
    assert!(language.is_failed());
}

#[test]
fn failure_is_cached_and_file_change_recovers() {
    let dir = std::env::temp_dir().join(format!("lucerna-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("json.wasm");
    std::fs::write(&path, b"garbage").unwrap();
    let language = Language::from_wasm_for_test("t-cached", &path, "(string) @s");
    assert!(!language.grammar_ready());
    assert!(SyntaxState::new(language.clone(), bundled()).is_err());
    assert!(SyntaxState::new(language.clone(), bundled()).is_err());
    assert!(language.is_failed());
    // The extension is updated in place: the new file (a different
    // length) loads rather than the cached failure.
    std::fs::copy(json_wasm(), &path).unwrap();
    assert!(SyntaxState::new(language.clone(), bundled()).is_ok());
    assert!(language.grammar_ready());
    let fresh = Language::from_wasm_for_test("t-cached-fresh", &path, "(string) @s");
    assert!(SyntaxState::new(fresh, bundled()).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_highlights_query_marks_extension_language_failed() {
    let language = Language::from_wasm_for_test("t-badquery", &json_wasm(), "(no_such_node) @x");
    let buffer = Buffer::from_bytes(br#"{"a": 1}"#, false).unwrap();
    let snap = buffer.snapshot();
    let mut state = SyntaxState::new(language.clone(), bundled()).expect("grammar loads");
    state.parse(&snap, None);
    assert!(
        state
            .highlights(&snap, 0..snap.line_count(), &|_| true)
            .is_empty()
    );
    assert!(language.is_failed());
}

#[test]
fn states_are_created_and_dropped_repeatedly() {
    let language = Language::from_wasm_for_test("t-pool", &json_wasm(), "(string) @string");
    for _ in 0..20 {
        let state = SyntaxState::new(language.clone(), bundled()).unwrap();
        drop(state);
    }
}

#[test]
fn styled_filter_drops_unstyled_captures() {
    let query = "(pair key: (_) @a)\n(pair key: (_) @b)";
    let language = Language::from_wasm_for_test_with("t-styled", &json_wasm(), query, true, &[]);
    let only_a = highlight(language.clone(), r#"{"a": 1}"#, &|c| c == "a");
    assert_eq!(only_a, vec![("\"a\"".to_string(), "a".to_string())]);
    let all = highlight(language, r#"{"a": 1}"#, &|_| true);
    assert_eq!(all, vec![("\"a\"".to_string(), "b".to_string())]);
}

#[test]
fn validate_wasm_grammar_rejects_garbage() {
    let bytes = std::fs::read(json_wasm()).unwrap();
    assert!(validate_wasm_grammar("json", &bytes).is_ok());
    assert!(validate_wasm_grammar("json", b"not wasm").is_err());
}

#[test]
fn validate_query_reports_bad_node() {
    let bytes = std::fs::read(json_wasm()).unwrap();
    assert!(validate_query("json", &bytes, "(string) @s").is_ok());
    let err = validate_query("json", &bytes, "(no_such_node) @x").unwrap_err();
    assert!(err.contains("no_such_node"), "{err}");
}

/// The registry injected language names resolve in.
fn bundled() -> std::sync::Arc<LanguageRegistry> {
    std::sync::Arc::new(LanguageRegistry::bundled())
}

#[test]
fn extension_query_files_are_read_on_first_use() {
    use lucerna_syntax::{Associations, ExtensionLanguage};
    let dir = std::env::temp_dir().join(format!("lucerna-lazy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let spec = ExtensionLanguage {
        extension_id: "lazy".into(),
        name: "Lazy".into(),
        dir: dir.clone(),
        grammar: Some(("json".into(), json_wasm())),
        path_suffixes: vec!["lazy".into()],
        first_line_pattern: None,
        hidden: false,
    };
    let registry = Arc::new(LanguageRegistry::build(&[spec], Associations::default()));
    let language = registry.language_by_name("Lazy").unwrap();
    assert!(!language.has_injections());
    // Written after the registry was built: building read no query file.
    std::fs::write(dir.join("highlights.scm"), "(string) @string\n").unwrap();
    std::fs::write(dir.join("injections.scm"), "").unwrap();
    assert!(language.has_injections());
    let buffer = Buffer::from_bytes(br#"{"a": 1}"#, false).unwrap();
    let snap = buffer.snapshot();
    let mut state = SyntaxState::new(language, registry).unwrap();
    state.parse(&snap, None);
    let spans = state.highlights(&snap, 0..1, &|_| true);
    assert!(spans.iter().any(|s| &*s.capture == "string"), "{spans:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bundled_languages_are_shared_across_registries() {
    use lucerna_syntax::Associations;
    let a = LanguageRegistry::bundled();
    let b = LanguageRegistry::build(&[], Associations::default());
    let rust = |r: &LanguageRegistry| r.language_by_name("rust").unwrap();
    assert!(Arc::ptr_eq(&rust(&a), &rust(&b)));
    let c = a.with_associations(Associations::default());
    assert!(Arc::ptr_eq(&rust(&a), &rust(&c)));
}
