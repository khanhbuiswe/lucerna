use std::fs;
use std::path::{Path, PathBuf};

use lucerna_theme::{IconSource, IconThemeRegistry, lucide_icon_theme};

fn b(name: &'static str) -> Option<IconSource> {
    Some(IconSource::Builtin(name))
}

#[test]
fn lucide_matches_phase1_table() {
    let t = lucide_icon_theme();
    assert_eq!(t.name, "Lucide");
    assert_eq!(t.directory_icon("src", false), b("folder"));
    assert_eq!(t.directory_icon("src", true), b("folder-open"));
    assert_eq!(t.directory_icon("x.json", false), b("folder"));
    assert_eq!(t.chevron(false), b("chevron-right"));
    assert_eq!(t.chevron(true), b("chevron-down"));
    let cases: &[(&str, &str)] = &[
        ("Cargo.lock", "lock"),
        ("pnpm-lock.lock", "lock"),
        ("LICENSE", "scale"),
        ("LICENSE-MIT", "scale"),
        ("LICENSE.md", "scale"),
        ("COPYING", "scale"),
        (".gitignore", "git-branch"),
        (".gitattributes", "git-branch"),
        ("sub/.gitmodules", "git-branch"),
        ("package.json", "braces"),
        ("tsconfig.jsonc", "braces"),
        ("README.md", "book-open"),
        ("notes.markdown", "book-open"),
        ("Cargo.toml", "settings"),
        ("ci.yaml", "settings"),
        ("ci.YML", "settings"),
        ("build.sh", "terminal"),
        ("x.bash", "terminal"),
        ("x.zsh", "terminal"),
        ("logo.png", "image"),
        ("a.jpg", "image"),
        ("a.jpeg", "image"),
        ("a.gif", "image"),
        ("icon.svg", "image"),
        ("a.webp", "image"),
        ("favicon.ico", "image"),
        ("Makefile", "file"),
        ("notes.txt", "file"),
        ("", "file"),
        ("src/main.RS", "file-code"),
        (".json", "file"),
        ("lock", "file"),
        ("lock.txt", "file"),
        ("md.orig", "file"),
        ("json", "file"),
        ("rs", "file"),
        (".foo.json", "braces"),
    ];
    for (name, icon) in cases {
        assert_eq!(t.file_icon(Path::new(name)), b(icon), "{name}");
    }
    for ext in [
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "c", "h", "cpp", "java", "kt",
        "swift", "rb", "php", "cs", "html", "css", "scss",
    ] {
        assert_eq!(
            t.file_icon(Path::new(&format!("src/x.{ext}"))),
            b("file-code"),
            "{ext}"
        );
    }
}

fn write_theme(dir: &Path, json: &str) -> (String, PathBuf, PathBuf) {
    let icons = dir.join("icons");
    fs::create_dir_all(&icons).unwrap();
    fs::create_dir_all(icons.join("sub")).unwrap();
    for n in ["docker", "tsdef", "ts", "dir", "dir-open", "named", "def"] {
        fs::write(icons.join(format!("{n}.svg")), "<svg/>").unwrap();
    }
    let f = dir.join("theme.json");
    fs::write(&f, json).unwrap();
    ("ext".to_string(), f, icons)
}

fn file_name(src: Option<IconSource>) -> String {
    match src {
        Some(IconSource::File(p)) => p.file_name().unwrap().to_string_lossy().into_owned(),
        other => panic!("expected a file icon, got {other:?}"),
    }
}

const STEMS: &str = r#"{ "name": "T", "author": "a", "themes": [{
  "name": "Stems", "appearance": "dark",
  "file_stems": {"Dockerfile": "docker"},
  "file_suffixes": {"d.ts": "tsdef", "ts": "ts"},
  "file_icons": {
    "docker": {"path": "icons/docker.svg"},
    "tsdef": {"path": "icons/tsdef.svg"},
    "ts": {"path": "icons/ts.svg"}
  },
  "unknown_key": 1 }] }"#;

#[test]
fn stem_then_suffix_order() {
    let dir = tempfile::tempdir().unwrap();
    // Paths are relative to the icons root, which holds the svgs directly.
    let json = STEMS.replace("icons/", "");
    let (id, f, root) = write_theme(dir.path(), &json);
    let (reg, warnings) = IconThemeRegistry::build(&[(id, f, root)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let t = reg.resolve("Stems");
    assert_eq!(t.name(), "Stems");
    let f = |n: &str| file_name(Some(t.file(Path::new(n))));
    assert_eq!(f("Dockerfile"), "docker.svg");
    assert_eq!(f("dockerfile"), "docker.svg");
    assert_eq!(f("x.d.ts"), "tsdef.svg");
    assert_eq!(f("x.ts"), "ts.svg");
}

#[test]
fn missing_type_falls_back_to_lucide() {
    let dir = tempfile::tempdir().unwrap();
    let json = STEMS.replace("icons/", "");
    let (id, f, root) = write_theme(dir.path(), &json);
    let (reg, _) = IconThemeRegistry::build(&[(id, f, root)]);
    let t = reg.resolve("Stems");
    // No `rs` mapping and no `default` type: Lucide answers.
    assert_eq!(t.file(Path::new("a.rs")), IconSource::Builtin("file-code"));
    assert_eq!(t.file(Path::new("a.zzz")), IconSource::Builtin("file"));
    assert_eq!(t.chevron(true), IconSource::Builtin("chevron-down"));
    assert_eq!(t.directory("src", false), IconSource::Builtin("folder"));
}

#[test]
fn named_directory_then_generic_then_lucide() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{ "name": "T", "themes": [{ "name": "Dirs", "appearance": "light",
      "directory_icons": {"collapsed": "dir.svg"},
      "named_directory_icons": {"src": {"collapsed": "named.svg"}} }] }"#;
    let (id, f, root) = write_theme(dir.path(), json);
    let (reg, warnings) = IconThemeRegistry::build(&[(id, f, root)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let t = reg.resolve("Dirs");
    assert_eq!(file_name(Some(t.directory("src", false))), "named.svg");
    assert_eq!(file_name(Some(t.directory("lib", false))), "dir.svg");
    // `expanded` is absent everywhere: Lucide.
    assert_eq!(t.directory("lib", true), IconSource::Builtin("folder-open"));
}

#[test]
fn escaping_icon_path_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{ "name": "T", "themes": [{ "name": "Esc", "appearance": "dark",
      "file_stems": {"a": "bad1", "b": "bad2", "c": "good"},
      "file_icons": {
        "bad1": {"path": "../../etc/x.svg"},
        "bad2": {"path": "/abs.svg"},
        "good": {"path": "docker.svg"} } }] }"#;
    let (id, f, root) = write_theme(dir.path(), json);
    let (reg, warnings) = IconThemeRegistry::build(&[(id, f, root)]);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    let t = reg.resolve("Esc");
    assert_eq!(t.file(Path::new("a")), IconSource::Builtin("file"));
    assert_eq!(t.file(Path::new("b")), IconSource::Builtin("file"));
    assert_eq!(file_name(Some(t.file(Path::new("c")))), "docker.svg");
}

#[test]
fn unknown_theme_name_resolves_to_seti() {
    let (reg, warnings) = IconThemeRegistry::build(&[]);
    assert!(warnings.is_empty());
    assert_eq!(reg.names(), vec!["Seti".to_string(), "Lucide".to_string()]);
    assert_eq!(reg.resolve("Gone").name(), "Seti");
    assert!(matches!(
        reg.resolve("Nope").file(Path::new("a.json")),
        IconSource::Glyph { name: "json", .. }
    ));
}

#[test]
fn vs_seti_alias_resolves_to_seti() {
    let (reg, _) = IconThemeRegistry::build(&[]);
    assert_eq!(reg.resolve("vs-seti").name(), "Seti");
    assert_eq!(reg.resolve("Seti").name(), "Seti");
}

#[test]
fn lucide_still_resolvable() {
    let (reg, _) = IconThemeRegistry::build(&[]);
    let t = reg.resolve("Lucide");
    assert_eq!(t.name(), "Lucide");
    assert_eq!(t.file(Path::new("a.json")), IconSource::Builtin("braces"));
}

#[test]
fn extension_cannot_take_the_vs_seti_name() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{ "name": "T", "themes": [{ "name": "vs-seti", "appearance": "dark",
      "file_stems": {"a": "d"}, "file_icons": {"d": {"path": "docker.svg"}} }] }"#;
    let (id, f, root) = write_theme(dir.path(), json);
    let (reg, _) = IconThemeRegistry::build(&[(id, f, root)]);
    assert_eq!(reg.names(), vec!["Seti".to_string(), "Lucide".to_string()]);
    assert!(matches!(
        reg.resolve("vs-seti").file(Path::new("a")),
        IconSource::Glyph { .. }
    ));
}

#[test]
fn extension_cannot_replace_seti() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{ "name": "T", "themes": [{ "name": "Seti", "appearance": "dark",
      "file_stems": {"a": "d"}, "file_icons": {"d": {"path": "docker.svg"}} }] }"#;
    let (id, f, root) = write_theme(dir.path(), json);
    let (reg, _) = IconThemeRegistry::build(&[(id, f, root)]);
    assert_eq!(reg.names(), vec!["Seti".to_string(), "Lucide".to_string()]);
    // A real extension theme is listed once, after the built-ins.
    let dir2 = tempfile::tempdir().unwrap();
    let (id, f, root) = write_theme(dir2.path(), &STEMS.replace("icons/", ""));
    let (reg, _) = IconThemeRegistry::build(&[(id, f, root)]);
    assert_eq!(reg.names(), ["Seti", "Lucide", "Stems"]);
    assert!(matches!(
        reg.resolve("Seti").file(Path::new("a")),
        IconSource::Glyph { .. }
    ));
}

#[test]
fn inner_parent_dir_allowed_escape_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{ "name": "T", "themes": [{ "name": "P", "appearance": "dark",
      "file_stems": {"a": "ok", "b": "bad"},
      "file_icons": {
        "ok": {"path": "sub/../docker.svg"},
        "bad": {"path": "../theme.json"} } }] }"#;
    let (id, f, root) = write_theme(dir.path(), json);
    let (reg, warnings) = IconThemeRegistry::build(&[(id, f, root)]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let t = reg.resolve("P");
    assert_eq!(file_name(Some(t.file(Path::new("a")))), "docker.svg");
    assert_eq!(t.file(Path::new("b")), IconSource::Builtin("file"));
}
