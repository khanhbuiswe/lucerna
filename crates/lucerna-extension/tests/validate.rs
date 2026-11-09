use lucerna_extension::validate::{check_icon_theme_json, check_theme_json};

fn write(dir: &std::path::Path, rel: &str, text: &str) -> std::path::PathBuf {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn theme_family_names_and_appearances() {
    let d = tempfile::tempdir().unwrap();
    let p = write(
        d.path(),
        "t.json",
        r#"{ "name": "F", "themes": [
            { "name": "A", "appearance": "light", "style": {} },
            { "name": "B", "appearance": "dark", }, ] }"#,
    );
    assert_eq!(
        check_theme_json(&p).unwrap(),
        [("A".into(), "light".into()), ("B".into(), "dark".into())]
    );
}

#[test]
fn theme_with_bad_shape_rejected() {
    let d = tempfile::tempdir().unwrap();
    for text in [
        "{ nope",
        r#"{ "themes": [] }"#,
        r#"{ "themes": [{ "name": "A", "appearance": "dim" }] }"#,
        r#"{ "themes": [{ "appearance": "dark" }] }"#,
    ] {
        let p = write(d.path(), "t.json", text);
        assert!(check_theme_json(&p).is_err(), "{text}");
    }
}

#[test]
fn icon_paths_escaping_or_missing_warn() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "icons/a.svg", "<svg/>");
    let p = write(
        d.path(),
        "icon_themes/i.json",
        r#"{ "themes": [{ "name": "I", "appearance": "dark",
            "chevron_icons": { "collapsed": "./icons/a.svg", "expanded": "/etc/hosts" },
            "file_icons": { "x": { "path": "icons/../../outside.svg" }, "y": { "path": "./icons/missing.svg" } } }] }"#,
    );
    let (themes, warnings) = check_icon_theme_json(&p, d.path()).unwrap();
    assert_eq!(themes, [("I".into(), "dark".into())]);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("/etc/hosts") && w.contains("outside"))
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("outside.svg") && w.contains("outside"))
    );
    assert!(warnings.iter().any(|w| w.contains("missing.svg")));
}
