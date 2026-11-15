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

#[test]
fn defaults_include_phase2_keys() {
    let s = SettingsStore::with_defaults();
    assert_eq!(
        s.get::<String>("workbench.iconTheme", None).as_deref(),
        Some("Seti")
    );
    assert_eq!(
        s.get::<bool>("workbench.editor.showIcons", None),
        Some(true)
    );
    assert_eq!(
        s.get::<bool>("extensions.autoCheckUpdates", None),
        Some(true)
    );
    let assoc = s
        .get::<BTreeMap<String, String>>("files.associations", None)
        .unwrap();
    assert!(assoc.is_empty());
}

#[test]
fn files_associations_merge_across_layers() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(Layer::User, r#"{"files.associations": {"*.a": "x"}}"#)
        .unwrap();
    s.set_layer(Layer::Workspace, r#"{"files.associations": {"*.b": "y"}}"#)
        .unwrap();
    let assoc = s
        .get::<BTreeMap<String, String>>("files.associations", None)
        .unwrap();
    assert_eq!(assoc.len(), 2);
    assert_eq!(assoc["*.a"], "x");
    assert_eq!(assoc["*.b"], "y");
}

#[test]
fn get_trusted_ignores_workspace_layer() {
    let mut s = SettingsStore::with_defaults();
    s.set_layer(
        Layer::Workspace,
        r#"{"lsp.x": "ws", "lsp.o": {"a": 1, "b": 1}, "[python]": {"lsp.py": "ws"}}"#,
    )
    .unwrap();
    assert_eq!(s.get_trusted::<String>("lsp.x", None), None);
    assert_eq!(s.get::<String>("lsp.x", None).as_deref(), Some("ws"));
    assert_eq!(s.get_trusted::<String>("lsp.py", Some("python")), None);

    s.set_layer(
        Layer::User,
        r#"{"lsp.x": "user", "lsp.o": {"b": 2}, "[python]": {"lsp.py": "user"}}"#,
    )
    .unwrap();
    assert_eq!(
        s.get_trusted::<String>("lsp.x", None).as_deref(),
        Some("user")
    );
    assert_eq!(
        s.get_trusted::<String>("lsp.py", Some("python")).as_deref(),
        Some("user")
    );
    // Objects still merge across Default and User, without Workspace.
    s.set_layer(Layer::Default, r#"{"lsp.o": {"a": 0, "c": 0}}"#)
        .unwrap();
    let merged: BTreeMap<String, u32> = s.get_trusted("lsp.o", None).unwrap();
    assert_eq!(
        merged,
        BTreeMap::from([("a".into(), 0), ("b".into(), 2), ("c".into(), 0)])
    );
}

#[test]
fn default_settings_have_phase3_keys() {
    let s = SettingsStore::with_defaults();
    assert_eq!(s.get::<bool>("lucerna.lsp.enabled", None), Some(true));
    assert_eq!(
        s.get::<Vec<String>>("lucerna.lsp.languageServers", None),
        Some(vec!["...".to_string()])
    );
    assert_eq!(
        s.get::<Vec<String>>("lucerna.lsp.languageServers", Some("python")),
        Some(vec![
            "basedpyright".to_string(),
            "!pyright".to_string(),
            "...".to_string()
        ])
    );
    assert_eq!(
        s.get::<serde_json::Value>("lsp", None),
        Some(serde_json::json!({}))
    );
    assert_eq!(
        s.get::<bool>("session.trustAllWorktrees", None),
        Some(false)
    );
    assert_eq!(s.get::<bool>("editor.hover.enabled", None), Some(true));
    assert_eq!(s.get::<u32>("editor.hover.delay", None), Some(300));
    assert_eq!(s.get::<bool>("editor.hover.sticky", None), Some(true));
    assert_eq!(s.get::<u32>("editor.hover.hidingDelay", None), Some(300));
    for k in [
        "multipleDefinitions",
        "multipleTypeDefinitions",
        "multipleImplementations",
        "multipleDeclarations",
        "multipleReferences",
    ] {
        assert_eq!(
            s.get::<String>(&format!("editor.gotoLocation.{k}"), None)
                .as_deref(),
            Some("peek"),
            "{k}"
        );
    }
    assert_eq!(
        s.get::<bool>("lucerna.diagnostics.gutterIcons", None),
        Some(true)
    );
    assert_eq!(
        s.get::<bool>("lucerna.diagnostics.inline.enabled", None),
        Some(false)
    );
    assert_eq!(s.get::<bool>("breadcrumbs.enabled", None), Some(true));
    assert_eq!(
        s.get::<String>("breadcrumbs.filePath", None).as_deref(),
        Some("on")
    );
    assert_eq!(
        s.get::<String>("breadcrumbs.symbolPath", None).as_deref(),
        Some("on")
    );
    assert_eq!(s.get::<bool>("breadcrumbs.icons", None), Some(true));
}
