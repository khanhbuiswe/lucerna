use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_syntax::{
    Associations, ExtensionLanguage, Language, LanguageRegistry, ParseOutcome, SyntaxState,
};
use lucerna_text::Buffer;

fn json_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm/json.wasm")
}

/// A fresh directory holding the given `.scm` files.
fn dir_with(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lucerna-layers-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn ext(extension_id: &str, name: &str, suffixes: &[&str]) -> ExtensionLanguage {
    ExtensionLanguage {
        extension_id: extension_id.to_string(),
        name: name.to_string(),
        dir: dir_with(
            &format!("{extension_id}-{}", name.replace(' ', "_")),
            &[("highlights.scm", "(string) @string")],
        ),
        grammar: Some(("json".to_string(), json_wasm())),
        path_suffixes: suffixes.iter().map(|s| s.to_string()).collect(),
        first_line_pattern: None,
        hidden: false,
    }
}

fn assoc(pairs: &[(&str, &str)]) -> Associations {
    Associations::new(
        &pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn id_of(registry: &LanguageRegistry, path: &str) -> Option<String> {
    registry
        .language_for_file(Path::new(path), "")
        .map(|l| l.id.to_string())
}

#[test]
fn bundled_equals_build_without_extensions() {
    let a = LanguageRegistry::bundled();
    let b = LanguageRegistry::build(&[], Associations::default());
    let ids = |r: &LanguageRegistry| {
        r.languages()
            .iter()
            .map(|l| l.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&a), ids(&b));
    assert_eq!(id_of(&b, "a.rs").as_deref(), Some("rust"));
}

#[test]
fn extension_suffix_beats_bundled() {
    let registry = LanguageRegistry::build(
        &[ext("zed-json", "Zed Json", &["json"])],
        Associations::default(),
    );
    assert_eq!(id_of(&registry, "a.json").as_deref(), Some("zed-json"));
    assert_eq!(id_of(&registry, "a.rs").as_deref(), Some("rust"));
}

#[test]
fn exact_filename_beats_suffix() {
    let registry = LanguageRegistry::build(
        &[ext("lockfmt", "Lockfmt", &["Cargo.lock"])],
        Associations::default(),
    );
    assert_eq!(id_of(&registry, "Cargo.lock").as_deref(), Some("lockfmt"));
    assert_eq!(id_of(&registry, "Other.toml").as_deref(), Some("toml"));
}

#[test]
fn longest_suffix_wins() {
    let registry = LanguageRegistry::build(
        &[
            ext("a", "Plain Ts", &["ts"]),
            ext("b", "Decl Ts", &["d.ts"]),
        ],
        Associations::default(),
    );
    assert_eq!(id_of(&registry, "x.d.ts").as_deref(), Some("decl-ts"));
    assert_eq!(id_of(&registry, "x.ts").as_deref(), Some("plain-ts"));
}

#[test]
fn tie_breaks_on_extension_id() {
    let registry = LanguageRegistry::build(
        &[ext("a", "Foo A", &["foo"]), ext("b", "Foo B", &["foo"])],
        Associations::default(),
    );
    assert_eq!(id_of(&registry, "x.foo").as_deref(), Some("foo-b"));
    // Input order does not matter.
    let registry = LanguageRegistry::build(
        &[ext("b", "Foo B", &["foo"]), ext("a", "Foo A", &["foo"])],
        Associations::default(),
    );
    assert_eq!(id_of(&registry, "x.foo").as_deref(), Some("foo-b"));
}

#[test]
fn associations_win_over_everything() {
    let registry = LanguageRegistry::build(
        &[ext("zed-json", "Zed Json", &["json"])],
        assoc(&[("*.json", "markdown")]),
    );
    assert_eq!(id_of(&registry, "a.json").as_deref(), Some("markdown"));
    assert_eq!(id_of(&registry, "dir/a.json").as_deref(), Some("markdown"));

    // A pattern with `/` matches the relative path.
    let registry = LanguageRegistry::build(&[], assoc(&[("docs/*.txt", "markdown")]));
    assert_eq!(id_of(&registry, "docs/a.txt").as_deref(), Some("markdown"));
    assert_eq!(id_of(&registry, "other/a.txt"), None);

    // The longer glob wins.
    let registry = LanguageRegistry::build(
        &[],
        assoc(&[("*.json", "markdown"), ("tsconfig*.json", "yaml")]),
    );
    assert_eq!(id_of(&registry, "tsconfig.json").as_deref(), Some("yaml"));
    assert_eq!(id_of(&registry, "a.json").as_deref(), Some("markdown"));
}

#[test]
fn invalid_glob_and_unknown_target_are_skipped() {
    let registry = LanguageRegistry::build(
        &[],
        assoc(&[("[", "markdown"), ("*.json", "no-such-language")]),
    );
    assert_eq!(id_of(&registry, "a.json").as_deref(), Some("json"));
}

#[test]
fn association_by_extension_language_name() {
    let registry = LanguageRegistry::build(
        &[ext("zed-json", "Zed Json", &["zedj"])],
        assoc(&[("*.zz", "zed json")]),
    );
    assert_eq!(id_of(&registry, "a.zz").as_deref(), Some("zed-json"));
}

#[test]
fn first_line_pattern_only_when_nothing_else() {
    let mut zz = ext("zz", "Zz", &["zzext"]);
    zz.first_line_pattern = Some(r"^#!.*\bzz\b".to_string());
    let registry = LanguageRegistry::build(&[zz], Associations::default());
    let line = "#!/usr/bin/env zz";
    let lookup = |path: &str| {
        registry
            .language_for_file(Path::new(path), line)
            .map(|l| l.id.to_string())
    };
    assert_eq!(lookup("script").as_deref(), Some("zz"));
    assert_eq!(lookup("a.json").as_deref(), Some("json"));
    assert_eq!(
        registry
            .language_for_file(Path::new("script"), "#!/bin/sh")
            .map(|l| l.id.to_string()),
        None
    );
    // `language_for_path` never looks at the first line.
    assert!(registry.language_for_path(Path::new("script")).is_none());
}

#[test]
fn first_line_is_cut_at_256_bytes() {
    let mut zz = ext("zz", "Zz", &["zzext"]);
    zz.first_line_pattern = Some("needle".to_string());
    let registry = LanguageRegistry::build(&[zz], Associations::default());
    let late = format!("{}needle", "é".repeat(200));
    assert!(registry.language_for_file(Path::new("x"), &late).is_none());
    let early = format!("{}needle", "a".repeat(200));
    assert!(registry.language_for_file(Path::new("x"), &early).is_some());
}

#[test]
fn hidden_language_not_matched_by_path_but_by_name() {
    let mut inline = ext("md-inline", "Hidden Inline", &["hid"]);
    inline.hidden = true;
    inline.first_line_pattern = Some("hid".to_string());
    let registry = LanguageRegistry::build(&[inline], assoc(&[("*.assoc", "hidden inline")]));
    assert!(
        registry
            .language_for_file(Path::new("a.hid"), "hid")
            .is_none()
    );
    assert!(registry.language_by_name("Hidden Inline").is_some());
    assert!(registry.language_by_name("hidden-inline").is_some());
    assert!(registry.language_by_name("foo.hid").is_some());
    // `files.associations` can still pick it.
    assert_eq!(
        id_of(&registry, "a.assoc").as_deref(),
        Some("hidden-inline")
    );
}

#[test]
fn language_by_name_order_and_dotted_input() {
    let registry = LanguageRegistry::bundled();
    let id = |n: &str| registry.language_by_name(n).map(|l| l.id.to_string());
    assert_eq!(id("Rust").as_deref(), Some("rust"));
    assert_eq!(id("typescriptreact").as_deref(), Some("typescriptreact"));
    assert_eq!(id("rs").as_deref(), Some("rust"));
    assert_eq!(id("foo.rs").as_deref(), Some("rust"));
    assert_eq!(id("nonexistent"), None);
}

#[test]
fn suffix_matching_is_case_insensitive() {
    let registry = LanguageRegistry::bundled();
    assert_eq!(id_of(&registry, "README.MD").as_deref(), Some("markdown"));
    assert_eq!(id_of(&registry, "X.JSON").as_deref(), Some("json"));
    assert_eq!(id_of(&registry, "cargo.LOCK").as_deref(), Some("toml"));
}

#[test]
fn extension_id_replaces_bundled() {
    let registry = LanguageRegistry::build(&[ext("x", "JSON", &["json"])], Associations::default());
    let jsons: Vec<_> = registry
        .languages()
        .iter()
        .filter(|l| &*l.id == "json")
        .collect();
    assert_eq!(jsons.len(), 1);
    assert!(matches!(
        jsons[0].origin(),
        lucerna_syntax::LanguageOrigin::Extension { .. }
    ));
}

fn captured(language: &Arc<Language>, text: &str) -> Vec<(String, String)> {
    let buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
    let snap = buffer.snapshot();
    let mut state = SyntaxState::new(language.clone(), bundled()).expect("grammar loads");
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    state
        .highlights(&snap, 0..snap.line_count(), &|_| true)
        .into_iter()
        .map(|s| (text[s.range].to_string(), s.capture.to_string()))
        .collect()
}

fn language_with(tag: &str, highlights: &str) -> Arc<Language> {
    let mut spec = ext(tag, tag, &["x"]);
    spec.dir = dir_with(tag, &[("highlights.scm", highlights)]);
    let registry = LanguageRegistry::build(&[spec], Associations::default());
    registry.language_by_name(tag).unwrap()
}

#[test]
fn has_parent_predicate() {
    let text = r#"{"k": ["v"]}"#;
    let yes = language_with("hp-yes", "((string) @x (#has-parent? @x pair))");
    let spans = captured(&yes, text);
    assert_eq!(spans, vec![("\"k\"".to_string(), "x".to_string())]);

    let no = language_with("hp-no", "((string) @x (#not-has-parent? @x pair))");
    let spans = captured(&no, text);
    assert_eq!(spans, vec![("\"v\"".to_string(), "x".to_string())]);

    let any = language_with("hp-any", "((string) @x (#has-parent? @x pair array))");
    assert_eq!(captured(&any, text).len(), 2);
}

#[test]
fn unknown_predicate_disables_pattern() {
    let language = language_with(
        "unk-pred",
        "((string) @x (#zed-only? @x))\n(number) @number",
    );
    let spans = captured(&language, r#"{"a": 1}"#);
    assert_eq!(spans, vec![("1".to_string(), "number".to_string())]);
    assert!(!language.is_failed());
}

#[test]
fn injections_query_is_stored() {
    let registry = LanguageRegistry::bundled();
    assert!(
        registry
            .language_by_name("markdown")
            .unwrap()
            .has_injections()
    );
    assert!(registry.language_by_name("html").unwrap().has_injections());
    assert!(!registry.language_by_name("rust").unwrap().has_injections());
    let mut spec = ext("inj", "Inj", &["inj"]);
    spec.dir = dir_with(
        "inj",
        &[
            ("highlights.scm", "(string) @string"),
            (
                "injections.scm",
                "((string) @injection.content (#set! injection.language \"json\"))",
            ),
        ],
    );
    let registry = LanguageRegistry::build(&[spec], Associations::default());
    assert!(registry.language_by_name("Inj").unwrap().has_injections());
}

/// The registry injected language names resolve in.
fn bundled() -> std::sync::Arc<LanguageRegistry> {
    std::sync::Arc::new(LanguageRegistry::bundled())
}
