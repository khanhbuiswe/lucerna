//! The Seti theme against VS Code's real data (vs-seti-icon-theme.json plus
//! the language contributions): glyph and dark colour per file.

use std::path::Path;

use lucerna_theme::{IconSource, IconThemeRegistry, seti_glyph_names, seti_icon_theme};

fn glyph(name: &str) -> (&'static str, u32) {
    match seti_icon_theme().file_icon(Path::new(name)) {
        Some(IconSource::Glyph { name, dark, .. }) => (name, dark),
        other => panic!("{name}: expected a glyph, got {other:?}"),
    }
}

#[test]
fn seti_matches_vscode_table() {
    let table: &[(&str, &str, u32)] = &[
        ("app.ts", "typescript", 0x519aba),
        ("a.spec.ts", "typescript", 0xe37933),
        ("a.test.ts", "typescript", 0xe37933),
        ("a.d.ts", "typescript", 0x519aba),
        ("Dockerfile", "docker", 0x519aba),
        ("DOCKERFILE", "docker", 0x519aba),
        ("Dockerfile.dev", "docker", 0x519aba),
        ("docker-compose.yml", "docker", 0xf55385),
        (".env", "config", 0x6d8086),
        (".env.example", "config", 0x6d8086),
        (".env.local", "config", 0x6d8086),
        (".gitignore", "git", 0x41535b),
        ("Cargo.toml", "config", 0x6d8086),
        ("README.md", "info", 0x519aba),
        ("Readme.MD", "info", 0x519aba),
        ("LICENSE", "license", 0xcbcb41),
        ("main.rs", "rust", 0x6d8086),
        ("package.json", "json", 0xcbcb41),
        ("tsconfig.json", "tsconfig", 0x519aba),
        ("tsconfig.app.json", "json", 0xcbcb41),
        ("Makefile", "makefile", 0xe37933),
        ("notes.txt", "default", 0xd4d7d6),
        ("archive.tar.gz", "default", 0xd4d7d6),
        ("noext", "default", 0xd4d7d6),
        ("weird.zzz", "default", 0xd4d7d6),
    ];
    for (path, g, dark) in table {
        assert_eq!(glyph(path), (*g, *dark), "{path}");
    }
    // Only the file name counts, not the directories before it.
    assert_eq!(glyph("src/deep/main.rs"), ("rust", 0x6d8086));
}

#[test]
fn seti_own_entries_beat_language_entries() {
    // `tsconfig.json` is in Seti's file names (tsconfig, blue) and also a
    // language file name (json, yellow): Seti's own entry wins.
    assert_eq!(glyph("tsconfig.json"), ("tsconfig", 0x519aba));
    // `Dockerfile` is only a language file name here; `.gitignore` only a
    // language extension (git, VS Code's `_git` colour).
    assert_eq!(glyph("dockerfile").0, "docker");
    assert_eq!(glyph(".gitignore"), ("git", 0x41535b));
}

#[test]
fn longest_extension_wins() {
    // `spec.ts` (orange), not `ts` (blue).
    assert_eq!(glyph("a.spec.ts"), ("typescript", 0xe37933));
    assert_eq!(glyph("a.ts"), ("typescript", 0x519aba));
    assert_eq!(glyph("a.b.c.ts"), ("typescript", 0x519aba));
}

#[test]
fn seti_has_no_folder_or_chevron_icons() {
    let t = seti_icon_theme();
    assert_eq!(t.name, "Seti");
    assert_eq!(t.directory_icon("src", false), None);
    assert_eq!(t.directory_icon("src", true), None);
    assert_eq!(t.chevron(false), None);
    assert_eq!(t.chevron(true), None);
    // So the resolved set falls back to Lucide for them.
    let (reg, _) = IconThemeRegistry::build(&[]);
    let r = reg.resolve("Seti");
    assert_eq!(r.directory("src", false), IconSource::Builtin("folder"));
    assert_eq!(r.chevron(true), IconSource::Builtin("chevron-down"));
}

#[test]
fn glyph_names_are_sorted_and_unique() {
    let names: Vec<_> = seti_glyph_names().collect();
    assert!(names.contains(&"default") && names.contains(&"typescript"));
    assert!(names.windows(2).all(|w| w[0] < w[1]));
}
