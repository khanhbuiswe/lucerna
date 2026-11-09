use std::collections::BTreeMap;

use lucerna_settings::{Layer, SettingsStore};

fn fs(s: &SettingsStore, lang: Option<&str>, key: &str) -> Option<u32> {
    s.get::<u32>(key, lang)
}

#[test]
fn defaults_present() {
    let s = SettingsStore::with_defaults();
    assert_eq!(fs(&s, None, "editor.fontSize"), Some(14));
    assert_eq!(
        s.get::<String>("editor.fontFamily", None).as_deref(),
        Some("Lilex")
    );
    assert_eq!(s.get::<f32>("editor.lineHeight", None), Some(1.6));
    assert_eq!(s.get::<bool>("editor.minimap.enabled", None), Some(false));
    assert_eq!(
        s.get::<u64>("lucerna.largeFile.maxBytes", None),
        Some(20971520)
    );
}

#[test]
fn user_overrides_default() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"editor.fontSize": 15}"#)
        .unwrap();
    assert_eq!(fs(&s, None, "editor.fontSize"), Some(15));
}

#[test]
fn workspace_overrides_user() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"editor.fontSize": 15}"#)
        .unwrap();
    s.set_layer(Layer::Workspace, r#"{"editor.fontSize": 16}"#)
        .unwrap();
    assert_eq!(fs(&s, None, "editor.fontSize"), Some(16));
}

#[test]
fn nested_form_supported() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"editor": {"tabSize": 2}}"#)
        .unwrap();
    assert_eq!(fs(&s, None, "editor.tabSize"), Some(2));
}

#[test]
fn language_override() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"[rust]": {"editor.tabSize": 2}}"#)
        .unwrap();
    assert_eq!(fs(&s, Some("rust"), "editor.tabSize"), Some(2));
    assert_eq!(fs(&s, None, "editor.tabSize"), Some(4));
    assert_eq!(fs(&s, Some("python"), "editor.tabSize"), Some(4));
}

/// R28: VS Code's order. Any language override beats every plain value.
#[test]
fn user_language_beats_workspace_plain() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"[rust]": {"editor.tabSize": 2}}"#)
        .unwrap();
    s.set_layer(Layer::Workspace, r#"{"editor.tabSize": 8}"#)
        .unwrap();
    assert_eq!(fs(&s, Some("rust"), "editor.tabSize"), Some(2));
    assert_eq!(fs(&s, Some("python"), "editor.tabSize"), Some(8));
}

/// Workspace[lang], User[lang], Default[lang], Workspace, User, Default:
/// each value shows once every layer above it is taken away.
#[test]
fn precedence_is_language_layers_then_plain_layers() {
    let order = [
        (Layer::Workspace, true, 1),
        (Layer::User, true, 2),
        (Layer::Default, true, 3),
        (Layer::Workspace, false, 4),
        (Layer::User, false, 5),
        (Layer::Default, false, 6),
    ];
    for winner in 0..order.len() {
        let mut layers: [Vec<String>; 3] = Default::default();
        for &(layer, lang, value) in &order[winner..] {
            let entry = if lang {
                format!(r#""[go]": {{"editor.tabSize": {value}}}"#)
            } else {
                format!(r#""editor.tabSize": {value}"#)
            };
            layers[layer as usize].push(entry);
        }
        let mut s = SettingsStore::default();
        for (layer, entries) in [Layer::Default, Layer::User, Layer::Workspace]
            .into_iter()
            .zip(&layers)
        {
            s.set_layer(layer, &format!("{{{}}}", entries.join(",")))
                .unwrap();
        }
        assert_eq!(
            fs(&s, Some("go"), "editor.tabSize"),
            Some(order[winner].2),
            "with {:?}",
            &order[winner..]
        );
    }
}

/// R26: object settings merge across layers, the higher layer winning
/// per key (this used to be `objects_replace_not_merge`).
#[test]
fn objects_merge_across_layers() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"files.exclude": {"**/dist": true}}"#)
        .unwrap();
    let m: BTreeMap<String, bool> = s.get("files.exclude", None).unwrap();
    assert_eq!(
        m,
        BTreeMap::from([
            ("**/.DS_Store".into(), true),
            ("**/.git".into(), true),
            ("**/dist".into(), true),
        ])
    );
}

#[test]
fn user_exclude_keeps_the_default_git_exclude() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(
        Layer::User,
        r#"{"files.exclude": {"**/node_modules": true}}"#,
    )
    .unwrap();
    let m: BTreeMap<String, bool> = s.get("files.exclude", None).unwrap();
    assert_eq!(m.get("**/.git"), Some(&true));
    assert_eq!(m.get("**/node_modules"), Some(&true));
}

#[test]
fn false_in_a_higher_layer_turns_a_lower_glob_off() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"files.exclude": {"**/.DS_Store": false}}"#)
        .unwrap();
    s.set_layer(
        Layer::Workspace,
        r#"{"search.exclude": {"**/target": false, "**/out": true}}"#,
    )
    .unwrap();
    let files: BTreeMap<String, bool> = s.get("files.exclude", None).unwrap();
    assert_eq!(files.get("**/.DS_Store"), Some(&false));
    assert_eq!(files.get("**/.git"), Some(&true));
    let search: BTreeMap<String, bool> = s.get("search.exclude", None).unwrap();
    assert_eq!(
        search,
        BTreeMap::from([
            ("**/node_modules".into(), true),
            ("**/out".into(), true),
            ("**/target".into(), false),
        ])
    );
}

#[test]
fn objects_merge_deeply_and_a_non_object_replaces() {
    let mut s = SettingsStore::default();
    s.set_layer(Layer::Default, r#"{"a": {"x": {"p": 1, "q": 2}, "y": 1}}"#)
        .unwrap();
    s.set_layer(Layer::User, r#"{"a": {"x": {"q": 3}}}"#)
        .unwrap();
    let v: serde_json::Value = s.get("a", None).unwrap();
    assert_eq!(v, serde_json::json!({"x": {"p": 1, "q": 3}, "y": 1}));
    s.set_layer(Layer::Workspace, r#"{"a": 7}"#).unwrap();
    assert_eq!(s.get::<u32>("a", None), Some(7));
}

#[test]
fn comments_and_trailing_commas_ok() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, "// c\n{\"editor.fontSize\": 14,}")
        .unwrap();
    assert_eq!(fs(&s, None, "editor.fontSize"), Some(14));
}

#[test]
fn invalid_json_keeps_previous_layer_and_reports_line() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"editor.fontSize": 15}"#)
        .unwrap();
    let err = s
        .set_layer(Layer::User, "{\n\"a\": 1\n\"b\": 2\n}")
        .unwrap_err();
    assert_eq!(err.line, Some(3));
    assert_eq!(fs(&s, None, "editor.fontSize"), Some(15));
}

#[test]
fn missing_comma_inside_nested_value_and_comment_gaps() {
    let mut s = SettingsStore::with_defaults();
    let err = s.set_layer(Layer::User, "{\"a\": [1\n2]}").unwrap_err();
    assert_eq!(err.line, Some(2));
    s.set_layer(Layer::User, "{\"a\": 1 /* x, y */ // z\n, \"b\": 2}")
        .unwrap();
    assert!(
        s.set_layer(Layer::User, "{\"a\": 1 /* , */ \"b\": 2}")
            .is_err()
    );
}

#[test]
fn non_object_root_is_rejected() {
    let mut s = SettingsStore::with_defaults();
    assert!(s.set_layer(Layer::User, "[1]").is_err());
}
