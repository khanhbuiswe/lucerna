use std::path::Path;

use lucerna_syntax::{LanguageRegistry, ParseOutcome, SyntaxState, matching_bracket};
use lucerna_text::{Buffer, BufferSnapshot};

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn parsed(path: &str, text: &str) -> (SyntaxState, BufferSnapshot) {
    let lang = LanguageRegistry::bundled()
        .language_for_path(Path::new(path))
        .unwrap();
    let snap = snapshot(text);
    let mut state = SyntaxState::new(lang, bundled()).unwrap();
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    (state, snap)
}

#[test]
fn matching_bracket_nested() {
    let text = "f(a[0], {b: (c)})";
    let snap = snapshot(text);
    // Text scan (no tree).
    assert_eq!(matching_bracket(&snap, 1), Some(16));
    assert_eq!(matching_bracket(&snap, 16), Some(1));
    assert_eq!(matching_bracket(&snap, 3), Some(5));
    assert_eq!(matching_bracket(&snap, 8), Some(15));
    assert_eq!(matching_bracket(&snap, 12), Some(14));
    // Just after a bracket also matches it.
    assert_eq!(matching_bracket(&snap, 17), Some(1));
    assert_eq!(matching_bracket(&snap, 0), None);
    assert_eq!(matching_bracket(&snap, 2), Some(16));
    assert_eq!(matching_bracket(&snapshot("(("), 0), None);

    // Tree-based, Rust.
    let rust = "fn f() { let v = [(1), (2)]; }";
    let (state, snap) = parsed("a.rs", rust);
    let open = rust.find('[').unwrap();
    let close = rust.find(']').unwrap();
    assert_eq!(state.matching_bracket(&snap, open), Some(close));
    assert_eq!(state.matching_bracket(&snap, close), Some(open));
    let brace = rust.find('{').unwrap();
    assert_eq!(state.matching_bracket(&snap, brace), Some(rust.len() - 1));
    assert_eq!(state.matching_bracket(&snap, rust.len()), Some(brace));
}

#[test]
fn matching_bracket_ignores_bracket_in_string() {
    let text = "(\")\")";
    let (state, snap) = parsed("a.rs", text);
    assert_eq!(state.matching_bracket(&snap, 0), Some(4));
    assert_eq!(state.matching_bracket(&snap, 4), Some(0));
    assert_eq!(state.matching_bracket(&snap, 2), None);
}

#[test]
fn matching_bracket_text_scan_handles_multibyte() {
    let text = "(é)";
    let snap = snapshot(text);
    assert_eq!(matching_bracket(&snap, 0), Some(3));
    assert_eq!(matching_bracket(&snap, 3), Some(0));
    // Offset 2 is inside `é`; neither it nor the byte before is a bracket.
    assert_eq!(matching_bracket(&snap, 2), None);
}

#[test]
fn brackets_scm_pairs_used_when_present() {
    use lucerna_syntax::{Associations, ExtensionLanguage};
    let dir = std::env::temp_dir().join(format!("lucerna-brackets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("highlights.scm"), "(string) @string").unwrap();
    std::fs::write(dir.join("brackets.scm"), "(\"[\" @open \"]\" @close)").unwrap();
    let spec = ExtensionLanguage {
        extension_id: "br".into(),
        name: "Br".into(),
        dir,
        grammar: Some((
            "json".into(),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/wasm/json.wasm"),
        )),
        path_suffixes: vec!["br".into()],
        first_line_pattern: None,
        hidden: false,
        editing: Default::default(),
    };
    let registry = LanguageRegistry::build(&[spec], Associations::default());
    let lang = registry.language_by_name("Br").unwrap();
    let text = r#"{"a": [1, 2], "b": 3}"#;
    let snap = snapshot(text);
    let mut state = SyntaxState::new(lang, bundled()).unwrap();
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    let open = text.find('[').unwrap();
    let close = text.find(']').unwrap();
    assert_eq!(state.matching_bracket(&snap, open), Some(close));
    assert_eq!(state.matching_bracket(&snap, close), Some(open));
    // Just after the closing bracket also matches.
    assert_eq!(state.matching_bracket(&snap, close + 1), Some(open));
    // `{}` is not a pair for this language.
    assert_eq!(state.matching_bracket(&snap, 0), None);
    assert_eq!(state.matching_bracket(&snap, text.len() - 1), None);
}

/// The registry injected language names resolve in.
fn bundled() -> std::sync::Arc<LanguageRegistry> {
    std::sync::Arc::new(LanguageRegistry::bundled())
}
