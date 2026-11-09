use std::path::{Path, PathBuf};

use lucerna_extension::{check_snippet_body, check_snippets};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifests/snippets")
        .join(name)
}

#[test]
fn snippets_language_from_stem() {
    let r = check_snippets(&fixture("rust.json")).unwrap();
    assert_eq!(r.language.as_deref(), Some("rust"));
    assert_eq!(r.valid, 2);
    assert_eq!(r.errors.len(), 2, "{:?}", r.errors);
    let g = check_snippets(&fixture("Snippets.json")).unwrap();
    assert_eq!(g.language, None);
    assert_eq!(g.valid, 1);
}

#[test]
fn snippet_body_accepts_textmate_forms() {
    for b in [
        "plain",
        "$1 $0",
        "${1} ${2:default}",
        "${1:outer ${2:inner}}",
        "${1|a,b,c|}",
        "$TM_FILENAME ${TM_FILENAME} ${TM_FILENAME:fallback}",
        r"\$1 \} \\ costs \$5",
        "a } b",
        "$ alone",
    ] {
        assert!(check_snippet_body(b).is_ok(), "{b}");
    }
}

#[test]
fn snippet_body_rejects_unclosed() {
    for b in [
        "${1:oops",
        "${1",
        "${1|a,b}",
        "${1|a,b|",
        "${1:${2:x}",
        "${",
    ] {
        assert!(check_snippet_body(b).is_err(), "{b}");
    }
}

#[test]
fn snippet_array_body_joined() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("go.json");
    std::fs::write(
        &f,
        r#"{"a":{"body":["if ${1:x} {","  $0","}"]},"b":{"body":["${1:open","close}"]},"c":{"body":["ok","${bad"]}}"#,
    )
    .unwrap();
    let r = check_snippets(&f).unwrap();
    assert_eq!(r.language.as_deref(), Some("go"));
    // Lines are joined with \n before checking: b is valid across lines.
    assert_eq!(r.valid, 2);
    assert_eq!(r.errors.len(), 1);
}

#[test]
fn snippets_bad_file_is_err() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x.json");
    std::fs::write(&f, "[1,2]").unwrap();
    assert!(check_snippets(&f).is_err());
    assert!(check_snippets(&d.path().join("none.json")).is_err());
}
