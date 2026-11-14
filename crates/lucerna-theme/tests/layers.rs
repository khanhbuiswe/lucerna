use std::path::{Path, PathBuf};

use lucerna_theme::{Appearance, ThemeRegistry};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn family(theme: &str, bg: &str) -> String {
    format!(
        r##"{{"name":"F","themes":[{{"name":"{theme}","appearance":"dark","style":{{"editor.background":"{bg}"}}}}]}}"##
    )
}

fn editor_bg(reg: &ThemeRegistry, name: &str) -> lucerna_theme::Rgba {
    reg.get(name).unwrap().ui.editor_background
}

#[test]
fn user_beats_extension_beats_builtin() {
    let ext_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let ext = write(
        ext_dir.path(),
        "ext.json",
        &family("Lucerna Dark", "#ff0000ff"),
    );

    let (reg, errors) = ThemeRegistry::build(&[("ext".to_string(), ext.clone())], None);
    assert!(errors.is_empty());
    assert_eq!(editor_bg(&reg, "Lucerna Dark").r, 1.0);

    write(
        user_dir.path(),
        "mine.json",
        &family("Lucerna Dark", "#00ff00ff"),
    );
    let (reg, errors) = ThemeRegistry::build(&[("ext".to_string(), ext)], Some(user_dir.path()));
    assert!(errors.is_empty());
    let bg = editor_bg(&reg, "Lucerna Dark");
    assert!(bg.g == 1.0 && bg.r == 0.0);
}

#[test]
fn extension_clash_later_id_wins() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.json", &family("Shared", "#ff0000ff"));
    let b = write(dir.path(), "b.json", &family("Shared", "#0000ffff"));
    // Given out of order: sorting by id makes "b" load last.
    let (reg, errors) =
        ThemeRegistry::build(&[("b-ext".to_string(), b), ("a-ext".to_string(), a)], None);
    assert!(errors.is_empty());
    assert_eq!(editor_bg(&reg, "Shared").b, 1.0);
}

#[test]
fn jsonc_theme_parses() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "c.json",
        r##"{
  // a comment
  "name": "C",
  /* block */
  "themes": [
    {"name": "Commented", "appearance": "light", "style": {"text": "#112233ff",},},
  ],
}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(reg.get("Commented").unwrap().appearance, Appearance::Light);
}

#[test]
fn broken_file_reported_others_load() {
    let ext_dir = tempfile::tempdir().unwrap();
    let user_dir = tempfile::tempdir().unwrap();
    let bad = write(ext_dir.path(), "bad.json", "{ not json");
    let good = write(ext_dir.path(), "good.json", &family("Good", "#010203ff"));
    write(user_dir.path(), "bad2.json", "[");
    write(user_dir.path(), "ok.json", &family("Fine", "#010203ff"));
    let (reg, errors) = ThemeRegistry::build(
        &[("x".to_string(), bad.clone()), ("y".to_string(), good)],
        Some(user_dir.path()),
    );
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().any(|(p, _)| *p == bad));
    assert!(
        errors
            .iter()
            .any(|(p, _)| *p == user_dir.path().join("bad2.json"))
    );
    assert!(reg.get("Good").is_some() && reg.get("Fine").is_some());
}

#[test]
fn appearance_listing_and_default_dark() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "n.json",
        r#"{"name":"N","themes":[{"name":"NoAppearance","style":{}}]}"#,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty(), "{errors:?}");
    let list = reg.themes_with_appearance();
    assert!(list.contains(&("NoAppearance".to_string(), Appearance::Dark)));
    assert!(list.contains(&("Lucerna Light".to_string(), Appearance::Light)));
}
