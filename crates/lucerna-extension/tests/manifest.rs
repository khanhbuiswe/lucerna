use std::path::{Path, PathBuf};

use lucerna_extension::{ExtensionManifest, load_language_config, load_manifest, query_files};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifests")
        .join(name)
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn schema1_full_manifest() {
    let m = load_manifest(&fixture("schema1")).unwrap();
    assert_eq!(m.id, "full-ext");
    assert_eq!(m.name, "Full Extension");
    assert_eq!(m.version, "1.2.3");
    assert_eq!(m.schema_version, 1);
    assert_eq!(m.description.as_deref(), Some("Everything at once"));
    assert_eq!(m.authors.len(), 2);
    assert!(m.lib.is_some());
    assert_eq!(m.themes, vec![p("themes/night.json")]);
    assert_eq!(m.icon_themes, vec![p("icon_themes/icons.json")]);
    assert_eq!(m.languages, vec![p("languages/foo")]);
    assert_eq!(m.snippets, vec![p("snippets/foo.json")]);
    let g = &m.grammars["foo"];
    assert_eq!(g.rev, "0123456789abcdef0123456789abcdef01234567");
    assert_eq!(g.path.as_deref(), Some("grammars/foo"));
    assert!(m.language_servers.contains_key("foo-ls"));
    assert!(m.other.contains_key("context_servers"));
    assert!(m.other.contains_key("capabilities"));
}

#[test]
fn manifest_round_trips_through_toml() {
    let m = load_manifest(&fixture("schema1")).unwrap();
    let text = toml::to_string(&m).unwrap();
    let back: ExtensionManifest = toml::from_str(&text).unwrap();
    assert_eq!(back.id, m.id);
    assert_eq!(back.grammars["foo"].rev, m.grammars["foo"].rev);
    assert_eq!(back.themes, m.themes);
    assert_eq!(back.snippets, m.snippets);
    assert!(back.other.contains_key("context_servers"));
    assert!(back.lib.is_some());
}

#[test]
fn schema0_reads_grammars_toml_and_discovers_dirs() {
    let m = load_manifest(&fixture("schema0")).unwrap();
    assert_eq!(m.languages, vec![p("languages/bar")]);
    assert_eq!(m.themes, vec![p("themes/real.json")]);
    assert_eq!(m.grammars.len(), 1);
    let g = &m.grammars["bar"];
    assert_eq!(g.repository, "https://github.com/example/tree-sitter-bar");
    assert_eq!(g.rev, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    assert_eq!(g.path.as_deref(), Some("sub"));
}

#[test]
fn legacy_extension_json_uses_dir_name_as_id() {
    let m = load_manifest(&fixture("legacy")).unwrap();
    assert_eq!(m.id, "legacy");
    assert_eq!(m.name, "Legacy Theme");
    assert_eq!(m.schema_version, 0);
    assert_eq!(m.themes, vec![p("themes/l.json")]);
}

#[test]
fn defaults_discovered_and_deduplicated() {
    let m = load_manifest(&fixture("defaults")).unwrap();
    assert_eq!(m.themes, vec![p("themes/b.json"), p("themes/a.json")]);
    assert_eq!(m.languages, vec![p("languages/b"), p("languages/a")]);
    assert_eq!(m.snippets, vec![p("snippets.json")]);
}

fn write_manifest(body: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("extension.toml"), body).unwrap();
    d
}

#[test]
fn schema_version_2_rejected() {
    let d = write_manifest("id='a'\nname='A'\nversion='1'\nschema_version=2\n");
    let e = load_manifest(d.path()).unwrap_err();
    assert!(e.to_string().contains("schema_version"), "{e}");
}

#[test]
fn bad_grammar_name_rejected() {
    let d = write_manifest(
        "id='a'\nname='A'\nversion='1'\nschema_version=1\n[grammars.Foo-Bar]\nrepository='x'\nrev='y'\n",
    );
    let e = load_manifest(d.path()).unwrap_err();
    assert!(e.to_string().contains("Foo-Bar"), "{e}");
}

#[test]
fn missing_manifest_is_error() {
    let d = tempfile::tempdir().unwrap();
    assert!(load_manifest(d.path()).is_err());
}

#[test]
fn missing_required_field_is_error() {
    let d = write_manifest("id='a'\nname='A'\nschema_version=1\n");
    assert!(load_manifest(d.path()).is_err());
}

#[test]
fn commit_alias_for_rev() {
    let d = write_manifest(
        "id='a'\nname='A'\nversion='1'\nschema_version=1\n[grammars.x]\nrepository='r'\ncommit='c0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ff'\n",
    );
    assert_eq!(
        load_manifest(d.path()).unwrap().grammars["x"].rev,
        "c0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ff"
    );
}

#[test]
fn snippets_accepts_string() {
    let d = write_manifest("id='a'\nname='A'\nversion='1'\nschema_version=1\nsnippets='s.json'\n");
    assert_eq!(load_manifest(d.path()).unwrap().snippets, vec![p("s.json")]);
}

#[test]
fn unsupported_lists_lsp_and_lib() {
    let m = load_manifest(&fixture("schema1")).unwrap();
    let u = m.unsupported();
    assert_eq!(&u[..2], &["language_servers", "lib"]);
    assert!(u.contains(&"context_servers"));
    let m = load_manifest(&fixture("defaults")).unwrap();
    assert!(m.unsupported().is_empty());
}

#[test]
fn language_config_ignores_unknown_fields() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("config.toml"),
        "name='X'\ngrammar='x'\npath_suffixes=['x']\nfirst_line_pattern='^#!'\nline_comments=['# ']\nhidden=true\ntab_size=2\nblock_comment=['/*','*/']\nbrackets=[{start='(',end=')',close=true}]\nweird=1\n",
    )
    .unwrap();
    let c = load_language_config(d.path()).unwrap();
    assert_eq!(c.name, "X");
    assert_eq!(c.grammar.as_deref(), Some("x"));
    assert_eq!(c.first_line_pattern.as_deref(), Some("^#!"));
    assert!(c.hidden);
    assert_eq!(c.tab_size, Some(2));
    assert_eq!(c.line_comments, vec!["# ".to_string()]);
}

#[test]
fn query_files_warn_on_folds_scm() {
    let d = tempfile::tempdir().unwrap();
    for f in ["highlights.scm", "injections.scm", "folds.scm", "notes.txt"] {
        std::fs::write(d.path().join(f), "").unwrap();
    }
    let (files, warnings) = query_files(d.path());
    let kinds: Vec<&str> = files.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, vec!["highlights", "injections"]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("folds.scm"));
}

fn mk(extra: &str) -> tempfile::TempDir {
    write_manifest(&format!(
        "id='a'\nname='A'\nversion='1'\nschema_version=1\n{extra}"
    ))
}

#[test]
fn absolute_and_escaping_paths_rejected() {
    for extra in [
        "themes=['/etc/passwd']",
        "themes=['../../secret.json']",
        "themes=['a/../../b.json']",
        "languages=['']",
        "snippets='/x.json'",
        "icon_themes=['..']",
        "[grammars.x]\nrepository='r'\nrev='c'\npath='../x'\n",
    ] {
        let d = mk(extra);
        assert!(load_manifest(d.path()).is_err(), "{extra}");
    }
}

#[cfg(unix)]
#[test]
fn symlinked_discovery_entries_skipped() {
    let d = mk("");
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("secret.json"), "{}").unwrap();
    std::fs::create_dir(d.path().join("themes")).unwrap();
    std::os::unix::fs::symlink(
        out.path().join("secret.json"),
        d.path().join("themes/x.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        out.path().join("secret.json"),
        d.path().join("snippets.json"),
    )
    .unwrap();
    let m = load_manifest(d.path()).unwrap();
    assert!(m.themes.is_empty());
    assert!(m.snippets.is_empty());
}

#[test]
fn dot_prefixed_entries_normalised_and_deduplicated() {
    let d = mk("themes=['./themes/a.json','themes/a.json']\n");
    std::fs::create_dir(d.path().join("themes")).unwrap();
    std::fs::write(d.path().join("themes/a.json"), "{}").unwrap();
    let m = load_manifest(d.path()).unwrap();
    assert_eq!(m.themes, vec![p("themes/a.json")]);
}

#[test]
fn json_nulls_are_dropped() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("extension.json"),
        r#"{"name":"N","version":"1","description":null,"extra":{"a":null,"b":[null,1]},"z":null}"#,
    )
    .unwrap();
    let m = load_manifest(d.path()).unwrap();
    assert_eq!(m.description, None);
    assert!(m.other.contains_key("extra"));
    assert!(!m.other.contains_key("z"));
}

#[test]
fn option_like_or_short_rev_rejected() {
    for rev in [
        "--upload-pack=touch /tmp/lucerna-pwned; git-upload-pack",
        "-c",
        "abc",
        "main",
        "0123456789abcdef0123456789abcdef0123456",
        "0123456789abcdef0123456789abcdef012345678",
        "0123456789abcdef0123456789abcdef0123456g",
    ] {
        let d = write_manifest(&format!(
            "id='a'\nname='A'\nversion='1'\nschema_version=1\n[grammars.x]\nrepository='https://github.com/o/r'\nrev='{rev}'\n"
        ));
        let e = load_manifest(d.path()).unwrap_err();
        assert!(e.to_string().contains("rev"), "{rev}: {e}");
    }
}

#[test]
fn full_sha1_and_sha256_revs_accepted() {
    for rev in [
        "0123456789abcdef0123456789abcdef01234567",
        "0123456789ABCDEF0123456789ABCDEF01234567",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        let d = write_manifest(&format!(
            "id='a'\nname='A'\nversion='1'\nschema_version=1\n[grammars.x]\nrepository='https://github.com/o/r'\nrev='{rev}'\n"
        ));
        assert_eq!(load_manifest(d.path()).unwrap().grammars["x"].rev, rev);
    }
}
