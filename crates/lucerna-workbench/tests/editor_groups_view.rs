//! The editor area view: key commands, tab bar mouse handling, context keys
//! and startup focus, driven through GPUI's test platform.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use lucerna_settings::{Context, Keymap, Layer, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, OpenMode, Workbench};

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings,
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
    });
}

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx))
}

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

fn holds(ctx: &Context, when: &str) -> bool {
    parse_when(when).unwrap().eval(ctx)
}

fn context(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Context {
    cx.update(|window, cx| wb.read(cx).context_keys(window, cx))
}

fn open_path(wb: &Entity<Workbench>, path: &str, mode: OpenMode, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| wb.open_path(p(path), mode, cx));
}

/// `(active group, paths per group)`.
fn layout(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (usize, Vec<Vec<String>>) {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx).model();
        let paths = groups
            .groups()
            .iter()
            .map(|g| {
                g.tabs
                    .iter()
                    .map(|t| t.path.display().to_string())
                    .collect()
            })
            .collect();
        (groups.active_group(), paths)
    })
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn cmd_backslash_splits_and_cmd_1_focuses(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    open_path(&wb, "src/main.rs", OpenMode::Pinned, cx);

    cx.simulate_keystrokes("cmd-\\");
    let main = vec!["src/main.rs".to_string()];
    assert_eq!(layout(&wb, cx), (1, vec![main.clone(), main.clone()]));

    cx.simulate_keystrokes("cmd-1");
    assert_eq!(layout(&wb, cx).0, 0);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(layout(&wb, cx).0, 1);
    // No third group: focus stays put.
    cx.simulate_keystrokes("cmd-3");
    assert_eq!(layout(&wb, cx).0, 1);

    // cmd+w closes the copy and removes the emptied second group.
    cx.simulate_keystrokes("cmd-w");
    assert_eq!(layout(&wb, cx), (0, vec![main]));
}

#[gpui::test]
fn ctrl_tab_cycles_mru(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    open_path(&wb, "a.rs", OpenMode::Pinned, cx);
    open_path(&wb, "b.rs", OpenMode::Pinned, cx);
    let active = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let view = wb.read(cx).editor_groups().read(cx);
            view.model()
                .active_tab()
                .map(|t| t.path.display().to_string())
        })
    };
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(active(cx).as_deref(), Some("a.rs"));
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(active(cx).as_deref(), Some("b.rs"));
}

#[gpui::test]
fn open_path_respects_enable_preview_setting(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "workbench.editor.enablePreview": false }"#));
    let (wb, cx) = open(cx);
    open_path(&wb, "a.rs", OpenMode::Preview, cx);
    open_path(&wb, "b.rs", OpenMode::Preview, cx);
    assert_eq!(
        layout(&wb, cx).1,
        vec![vec!["a.rs".to_string(), "b.rs".to_string()]]
    );
}

#[gpui::test]
fn editor_focused_at_startup(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    draw(cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "editorFocus && !sideBarFocus"));
    // An `editorFocus` binding works without clicking first.
    let probe = Keymap::load(
        Keymap::default_bindings(),
        Some(r#"[{ "key": "cmd+u", "command": "workbench.action.toggleSidebarVisibility", "when": "editorFocus" }]"#),
    )
    .0;
    cx.update(|_, cx| cx.global_mut::<AppState>().keymap = probe);
    cx.simulate_keystrokes("cmd-u");
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));
}

#[gpui::test]
fn context_keys_follow_active_tab(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "resourceExtname == '' && editorLangId == ''"));

    open_path(&wb, "/tmp/project/src/lib.rs", OpenMode::Preview, cx);
    let ctx = context(&wb, cx);
    assert!(holds(
        &ctx,
        "resourceExtname == '.rs' && editorLangId == 'rust'"
    ));

    open_path(&wb, "/tmp/project/notes.unknownext", OpenMode::Preview, cx);
    let ctx = context(&wb, cx);
    assert!(holds(
        &ctx,
        "resourceExtname == '.unknownext' && editorLangId == ''"
    ));

    open_path(&wb, "/tmp/project/Makefile", OpenMode::Preview, cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "resourceExtname == ''"));

    // VS Code: a dotfile's whole name is its extension.
    open_path(&wb, "/tmp/project/.gitignore", OpenMode::Preview, cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "resourceExtname == '.gitignore'"));
}

#[gpui::test]
fn tab_bar_mouse_activates_and_closes(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    open_path(&wb, "a.rs", OpenMode::Pinned, cx);
    open_path(&wb, "b.rs", OpenMode::Pinned, cx);
    open_path(&wb, "c.rs", OpenMode::Preview, cx);
    draw(cx);
    let bounds = |name: &'static str, cx: &mut VisualTestContext| {
        cx.debug_bounds(name)
            .unwrap_or_else(|| panic!("no element {name}"))
    };

    // Clicking a tab activates it.
    let a = bounds("tab-0-0", cx);
    cx.simulate_click(a.center(), Modifiers::none());
    let active = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let view = wb.read(cx).editor_groups().read(cx);
            view.model()
                .active_tab()
                .map(|t| t.path.display().to_string())
        })
    };
    assert_eq!(active(cx).as_deref(), Some("a.rs"));

    // Middle-click closes a tab.
    draw(cx);
    let b = bounds("tab-0-1", cx);
    cx.simulate_mouse_down(b.center(), MouseButton::Middle, Modifiers::none());
    cx.simulate_mouse_up(b.center(), MouseButton::Middle, Modifiers::none());
    assert_eq!(
        layout(&wb, cx).1,
        vec![vec!["a.rs".to_string(), "c.rs".to_string()]]
    );

    // The close button of an inactive tab shows on hover and closes it.
    draw(cx);
    let c = bounds("tab-0-1", cx);
    cx.simulate_mouse_move(c.center(), None, Modifiers::none());
    draw(cx);
    let close = bounds("tab-close-0-1", cx);
    cx.simulate_click(close.center(), Modifiers::none());
    assert_eq!(layout(&wb, cx).1, vec![vec!["a.rs".to_string()]]);
}

#[gpui::test]
fn dragging_divider_resizes_groups(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    open_path(&wb, "a.rs", OpenMode::Pinned, cx);
    cx.simulate_keystrokes("cmd-\\");
    draw(cx);
    let widths = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).group_widths().to_vec())
    };
    assert_eq!(widths(cx), vec![0.5, 0.5]);

    let area = cx.debug_bounds("editor-groups").expect("editor groups");
    let sash = cx.debug_bounds("group-sash-0").expect("divider");
    let y = sash.center().y;
    cx.simulate_mouse_down(sash.center(), MouseButton::Left, Modifiers::none());
    let quarter = area.left() + area.size.width * 0.25;
    cx.simulate_mouse_move(
        point(sash.center().x + px(10.), y),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(point(quarter, y), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(quarter, y), MouseButton::Left, Modifiers::none());
    let w = widths(cx);
    assert!((w[0] - 0.25).abs() < 0.01, "{w:?}");
    assert!((w[0] + w[1] - 1.0).abs() < 1e-4, "{w:?}");
}

#[gpui::test]
fn registers_editor_group_commands(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    let ids: Vec<String> = wb.read_with(cx, |wb, _| {
        wb.commands().all().map(|(id, _)| id.to_string()).collect()
    });
    let ordinals = [
        "First", "Second", "Third", "Fourth", "Fifth", "Sixth", "Seventh", "Eighth",
    ];
    let expected = ordinals
        .iter()
        .map(|n| format!("workbench.action.focus{n}EditorGroup"))
        .chain(
            [
                "workbench.action.splitEditor",
                "workbench.action.closeActiveEditor",
                "workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup",
            ]
            .map(String::from),
        );
    for id in expected {
        assert!(ids.contains(&id), "missing {id}");
    }

    // cmd+8 focuses the eighth group once it exists.
    open_path(&wb, "a.rs", OpenMode::Pinned, cx);
    for _ in 0..7 {
        cx.simulate_keystrokes("cmd-\\");
    }
    cx.simulate_keystrokes("cmd-1 cmd-8");
    assert_eq!(layout(&wb, cx).0, 7);
}
