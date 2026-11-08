//! Workbench shell: layout state, command registry and VS Code–style key
//! dispatch, driven through GPUI's test platform.

use std::path::PathBuf;

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use lucerna_settings::{Context, Keymap, SettingsStore, parse_when};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, SidebarView, Workbench};

fn init(cx: &mut TestAppContext, user_keymap: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), user_keymap);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
    });
}

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx))
}

fn holds(ctx: &Context, when: &str) -> bool {
    parse_when(when).unwrap().eval(ctx)
}

fn context(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Context {
    cx.update(|window, cx| wb.read(cx).context_keys(window, cx))
}

#[gpui::test]
fn default_layout_shows_explorer(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    wb.read_with(cx, |wb, _| {
        assert!(wb.sidebar_visible());
        assert_eq!(wb.active_sidebar(), SidebarView::Explorer);
        assert_eq!(wb.sidebar_width(), px(260.));
        assert_eq!(wb.pending_chord_label(), None);
    });
}

#[gpui::test]
fn cmd_b_toggles_sidebar(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    cx.simulate_keystrokes("cmd-b");
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));
    cx.simulate_keystrokes("cmd-b");
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));
}

#[gpui::test]
fn user_unbinding_disables_cmd_b(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"[{ "key": "cmd+b", "command": "-workbench.action.toggleSidebarVisibility" }]"#),
    );
    let (wb, cx) = open(cx);
    cx.simulate_keystrokes("cmd-b");
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));
}

#[gpui::test]
fn chord_pending_shows_in_status_bar(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.pending_chord_label()),
        Some("⌘K".to_string())
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(wb.read_with(cx, |wb, _| wb.pending_chord_label()), None);
}

#[gpui::test]
fn chord_no_match_re_resolves_last_keystroke(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    // "cmd+k cmd+b" is unbound: the chord is dropped and "cmd+b" runs alone.
    cx.simulate_keystrokes("cmd-k cmd-b");
    wb.read_with(cx, |wb, _| {
        assert_eq!(wb.pending_chord_label(), None);
        assert!(!wb.sidebar_visible());
    });
}

#[gpui::test]
fn context_keys_reflect_focus(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);

    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "sideBarVisible"));
    assert!(!holds(&ctx, "sideBarFocus"));
    assert!(!holds(&ctx, "inQuickOpen || findWidgetVisible"));
    assert!(holds(&ctx, "resourceExtname == '' && editorLangId == ''"));

    wb.update_in(cx, |wb, window, cx| wb.focus_sidebar(window, cx));
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "sideBarFocus && explorerViewletFocus"));
    assert!(!holds(&ctx, "searchViewletFocus || editorFocus"));

    // Bound to a command no task has registered yet: a harmless no-op.
    cx.simulate_keystrokes("cmd-shift-f");
    wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.search", None, window, cx)
    });
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "sideBarFocus && searchViewletFocus"));
    assert!(!holds(&ctx, "explorerViewletFocus"));

    wb.update_in(cx, |wb, window, cx| wb.focus_editor_area(window, cx));
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "editorFocus && !sideBarFocus"));
}

#[gpui::test]
fn hiding_focused_sidebar_moves_focus_out(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    wb.update_in(cx, |wb, window, cx| wb.focus_sidebar(window, cx));
    cx.simulate_keystrokes("cmd-b");
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "!sideBarVisible && !sideBarFocus"));
}

#[gpui::test]
fn view_commands_show_and_switch_sidebar(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    cx.simulate_keystrokes("cmd-b");
    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.search", None, window, cx)
    });
    assert!(ran);
    wb.read_with(cx, |wb, _| {
        assert!(wb.sidebar_visible());
        assert_eq!(wb.active_sidebar(), SidebarView::Search);
    });
    cx.simulate_keystrokes("cmd-shift-e");
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.active_sidebar()),
        SidebarView::Explorer
    );
    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("no.such.command", None, window, cx)
    });
    assert!(!ran);
}

#[gpui::test]
fn registry_lists_shell_commands(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    let all: Vec<(String, String)> = wb.read_with(cx, |wb, _| {
        wb.commands()
            .all()
            .map(|(id, title)| (id.to_string(), title.to_string()))
            .collect()
    });
    for id in [
        "workbench.action.toggleSidebarVisibility",
        "workbench.view.explorer",
        "workbench.view.search",
    ] {
        let entry = all.iter().find(|(i, _)| i == id);
        assert!(entry.is_some_and(|(_, t)| !t.is_empty()), "missing {id}");
    }
}

#[gpui::test]
fn sidebar_width_is_clamped(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    wb.update(cx, |wb, cx| {
        wb.set_sidebar_width(px(50.), cx);
        assert_eq!(wb.sidebar_width(), px(160.));
        wb.set_sidebar_width(px(1000.), cx);
        assert_eq!(wb.sidebar_width(), px(600.));
        wb.set_sidebar_width(px(300.), cx);
        assert_eq!(wb.sidebar_width(), px(300.));
    });
}

#[gpui::test]
fn dragging_sash_resizes_sidebar(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    // Activity bar 48 px + sidebar 260 px: the sash is the 4 px strip ending at x = 308.
    let sash = point(px(306.), px(200.));
    cx.simulate_mouse_down(sash, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        point(px(330.), px(200.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(
        point(px(448.), px(200.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    assert_eq!(wb.read_with(cx, |wb, _| wb.sidebar_width()), px(400.));
    cx.simulate_mouse_move(
        point(px(60.), px(200.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(
        point(px(60.), px(200.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    assert_eq!(wb.read_with(cx, |wb, _| wb.sidebar_width()), px(160.));
}
