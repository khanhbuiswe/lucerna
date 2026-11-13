use lucerna_settings::{Context, ContextValue, Keymap, Resolution, parse_keys};

fn defaults() -> Keymap {
    Keymap::load(Keymap::default_bindings(), None).0
}

fn keys(s: &str) -> Vec<lucerna_settings::Keystroke> {
    parse_keys(s).unwrap()
}

fn command(r: Resolution) -> String {
    match r {
        Resolution::Command { command, .. } => command,
        other => panic!("expected command, got {other:?}"),
    }
}

#[test]
fn parses_modifiers() {
    let k = keys("cmd+shift+p");
    assert_eq!(k.len(), 1);
    assert!(k[0].cmd && k[0].shift && !k[0].ctrl && !k[0].alt);
    assert_eq!(k[0].key, "p");
}

#[test]
fn parses_chord() {
    assert_eq!(keys("cmd+k cmd+t").len(), 2);
    assert!(parse_keys("").is_err());
    assert!(parse_keys("cmd+").is_err());
}

#[test]
fn default_cmd_p_resolves() {
    let r = defaults().resolve(&keys("cmd+p"), &Context::default());
    assert_eq!(command(r), "workbench.action.quickOpen");
}

#[test]
fn chord_pending_then_command() {
    let km = defaults();
    let ctx = Context::default();
    assert_eq!(km.resolve(&keys("cmd+k"), &ctx), Resolution::Pending);
    let r = km.resolve(&keys("cmd+k cmd+t"), &ctx);
    assert_eq!(command(r), "workbench.action.selectTheme");
    assert_eq!(km.resolve(&keys("cmd+k cmd+x"), &ctx), Resolution::NoMatch);
}

#[test]
fn when_gates_binding() {
    let km = defaults();
    let mut ctx = Context::default();
    assert_eq!(km.resolve(&keys("escape"), &ctx), Resolution::NoMatch);
    ctx.set("findWidgetVisible", ContextValue::Bool(true));
    // Focus elsewhere (the Search panel): Esc is not the find widget's.
    assert_eq!(km.resolve(&keys("escape"), &ctx), Resolution::NoMatch);
    ctx.set("editorFocus", ContextValue::Bool(true));
    assert_eq!(
        command(km.resolve(&keys("escape"), &ctx)),
        "closeFindWidget"
    );
}

#[test]
fn later_binding_wins() {
    let user = r#"[{"key":"cmd+p","command":"foo"}]"#;
    let (km, err) = Keymap::load(Keymap::default_bindings(), Some(user));
    assert!(err.is_none());
    assert_eq!(
        command(km.resolve(&keys("cmd+p"), &Context::default())),
        "foo"
    );
}

#[test]
fn minus_removes_default() {
    let user = r#"[{"key":"cmd+b","command":"-workbench.action.toggleSidebarVisibility"}]"#;
    let (km, err) = Keymap::load(Keymap::default_bindings(), Some(user));
    assert!(err.is_none());
    assert_eq!(
        km.resolve(&keys("cmd+b"), &Context::default()),
        Resolution::NoMatch
    );
}

#[test]
fn minus_with_when_removes_only_matching_when() {
    let user = r#"[{"key":"escape","command":"-closeFindWidget","when":"editorFocus && findWidgetVisible"}]"#;
    let (km, _) = Keymap::load(Keymap::default_bindings(), Some(user));
    let mut ctx = Context::default();
    ctx.set("editorFocus", ContextValue::Bool(true));
    ctx.set("findWidgetVisible", ContextValue::Bool(true));
    assert_eq!(km.resolve(&keys("escape"), &ctx), Resolution::NoMatch);
    // A different `when` does not match, so the default stays.
    let user = r#"[{"key":"escape","command":"-closeFindWidget","when":"other"}]"#;
    let (km, _) = Keymap::load(Keymap::default_bindings(), Some(user));
    assert_eq!(
        command(km.resolve(&keys("escape"), &ctx)),
        "closeFindWidget"
    );
}

#[test]
fn broken_user_file_falls_back() {
    let (km, err) = Keymap::load(Keymap::default_bindings(), Some("["));
    assert!(err.is_some());
    assert_eq!(
        command(km.resolve(&keys("cmd+p"), &Context::default())),
        "workbench.action.quickOpen"
    );
}

#[test]
fn bad_user_entry_falls_back() {
    let (km, err) = Keymap::load(Keymap::default_bindings(), Some(r#"[{"key":"cmd+p"}]"#));
    assert!(err.is_some());
    assert_eq!(
        command(km.resolve(&keys("cmd+p"), &Context::default())),
        "workbench.action.quickOpen"
    );
}

#[test]
fn label_for_quick_open() {
    let km = defaults();
    assert_eq!(
        km.label_for("workbench.action.quickOpen"),
        Some("⌘P".into())
    );
    assert_eq!(
        km.label_for("workbench.action.selectTheme"),
        Some("⌘K ⌘T".into())
    );
    assert_eq!(
        km.label_for("workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup"),
        Some("⌃Tab".into())
    );
    assert_eq!(km.label_for("closeFindWidget"), Some("Esc".into()));
    assert_eq!(km.label_for("nope"), None);
}

#[test]
fn defaults_all_parse() {
    // The bundled asset must load without error and include cmd+1..8, not 9.
    let (km, err) = Keymap::load(Keymap::default_bindings(), None);
    assert!(err.is_none());
    let ctx = Context::default();
    assert_eq!(
        command(km.resolve(&keys("cmd+8"), &ctx)),
        "workbench.action.focusEighthEditorGroup"
    );
    assert_eq!(km.resolve(&keys("cmd+9"), &ctx), Resolution::NoMatch);
}

#[test]
fn text_inputs_keep_option_z_and_cmd_c() {
    let km = defaults();
    let mut ctx = Context::default();
    ctx.set("editorFocus", ContextValue::Bool(true));
    assert_eq!(
        command(km.resolve(&keys("alt+z"), &ctx)),
        "editor.action.toggleWordWrap"
    );
    assert_eq!(
        command(km.resolve(&keys("cmd+c"), &ctx)),
        "editor.action.clipboardCopyAction"
    );
    // The find widget's input sits inside the editor: the input gets them.
    ctx.set("textInputFocus", ContextValue::Bool(true));
    assert_eq!(km.resolve(&keys("alt+z"), &ctx), Resolution::NoMatch);
    assert_eq!(km.resolve(&keys("cmd+c"), &ctx), Resolution::NoMatch);
}

#[test]
fn keymap_has_phase3_bindings() {
    let km = defaults();
    let mut editor = Context::default();
    // A focused code view: editorTextFocus implies editorFocus.
    editor.set("editorFocus", ContextValue::Bool(true));
    editor.set("editorTextFocus", ContextValue::Bool(true));
    // F8 / ⇧F8 also work from the Search and References tabs.
    let mut other_tab = Context::default();
    other_tab.set("editorFocus", ContextValue::Bool(true));
    for (k, cmd) in [
        ("f8", "editor.action.marker.nextInFiles"),
        ("shift+f8", "editor.action.marker.prevInFiles"),
    ] {
        assert_eq!(command(km.resolve(&keys(k), &other_tab)), cmd, "{k}");
    }
    let cases = [
        ("f12", "editor.action.revealDefinition"),
        ("alt+f12", "editor.action.peekDefinition"),
        ("cmd+f12", "editor.action.goToImplementation"),
        ("shift+cmd+f12", "editor.action.peekImplementation"),
        ("shift+f12", "editor.action.goToReferences"),
        ("shift+alt+f12", "references-view.findReferences"),
        ("cmd+k f12", "editor.action.revealDefinitionAside"),
        ("cmd+k cmd+i", "editor.action.showHover"),
        ("f8", "editor.action.marker.nextInFiles"),
        ("shift+f8", "editor.action.marker.prevInFiles"),
        ("alt+f8", "editor.action.marker.next"),
        ("shift+alt+f8", "editor.action.marker.prev"),
        ("shift+cmd+o", "workbench.action.gotoSymbol"),
        ("shift+cmd+.", "breadcrumbs.focusAndSelect"),
    ];
    for (k, cmd) in cases {
        assert_eq!(command(km.resolve(&keys(k), &editor)), cmd, "{k}");
        // Editor actions need the editor focused.
        assert_eq!(
            km.resolve(&keys(k), &Context::default()),
            Resolution::NoMatch,
            "{k} without focus"
        );
    }
    let none = Context::default();
    assert_eq!(
        command(km.resolve(&keys("cmd+t"), &none)),
        "workbench.action.showAllSymbols"
    );
    assert_eq!(
        command(km.resolve(&keys("ctrl+-"), &none)),
        "workbench.action.navigateBack"
    );
    assert_eq!(
        command(km.resolve(&keys("ctrl+shift+-"), &none)),
        "workbench.action.navigateForward"
    );
    // f4 keeps its search meaning, and is the References view's when focused.
    assert_eq!(
        command(km.resolve(&keys("f4"), &none)),
        "search.action.focusNextSearchResult"
    );
    let mut refs = Context::default();
    refs.set("referencesViewFocus", ContextValue::Bool(true));
    assert_eq!(
        command(km.resolve(&keys("f4"), &refs)),
        "references-view.next"
    );
    assert_eq!(
        command(km.resolve(&keys("shift+f4"), &refs)),
        "references-view.prev"
    );
}

#[test]
fn markdown_bindings_parse() {
    let (km, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "{error:?}");
    let mut source = Context::default();
    source.set("editorLangId", ContextValue::Str("markdown".into()));
    assert_eq!(
        command(km.resolve(&keys("cmd+shift+v"), &source)),
        "markdown.togglePreview"
    );
    assert_eq!(
        command(km.resolve(&keys("cmd+k v"), &source)),
        "markdown.showPreviewToSide"
    );
    let mut preview = Context::default();
    preview.set("activeEditor", ContextValue::Str("markdownPreview".into()));
    assert_eq!(
        command(km.resolve(&keys("cmd+shift+v"), &preview)),
        "markdown.togglePreview"
    );
    // Elsewhere the keys do nothing.
    let mut rust = Context::default();
    rust.set("editorLangId", ContextValue::Str("rust".into()));
    assert_eq!(km.resolve(&keys("cmd+shift+v"), &rust), Resolution::NoMatch);
}
