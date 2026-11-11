//! Workbench shell: layout state, command registry and VS Code–style key
//! dispatch, driven through GPUI's test platform.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyView, Context as ViewContext, Entity, FocusHandle, KeyDownEvent, Modifiers, MouseButton,
    Render, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px,
};
use lucerna_settings::{Context, Keymap, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, SidebarView, StatusBarSide, Workbench};

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
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
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

    // Search is an editor tab: its focus is not the sidebar's.
    wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.search", None, window, cx)
    });
    draw(cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "searchViewletFocus && editorFocus"));
    assert!(!holds(&ctx, "sideBarFocus || explorerViewletFocus"));

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
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));
    // Search is an editor tab: the sidebar stays hidden.
    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.search", None, window, cx)
    });
    assert!(ran);
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));
    cx.simulate_keystrokes("cmd-shift-e");
    wb.read_with(cx, |wb, _| {
        assert!(wb.sidebar_visible());
        assert_eq!(wb.active_sidebar(), SidebarView::Explorer);
    });
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
    // No activity bar: the sidebar starts at x = 0, so its sash is the 4 px
    // strip ending at x = 260.
    let sash = point(px(258.), px(200.));
    cx.simulate_mouse_down(sash, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        point(px(280.), px(200.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(
        point(px(400.), px(200.)),
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

/// A focusable child view that counts renders and records keys it receives.
struct Probe {
    focus: FocusHandle,
    renders: usize,
    keys: Vec<String>,
}

impl Probe {
    fn new(cx: &mut TestAppContext) -> Entity<Probe> {
        cx.new(|cx| Probe {
            focus: cx.focus_handle(),
            renders: 0,
            keys: Vec::new(),
        })
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut ViewContext<Self>) -> impl IntoElement {
        self.renders += 1;
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|probe, ev: &KeyDownEvent, _, _| {
                probe.keys.push(ev.keystroke.key.clone());
            }))
            .size_full()
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn focus(probe: &Entity<Probe>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let handle = probe.read(cx).focus.clone();
        window.focus(&handle, cx);
    });
}

fn same(view: Option<&AnyView>, probe: &Entity<Probe>) -> bool {
    view.is_some_and(|v| v.entity_id() == probe.entity_id())
}

#[gpui::test]
fn unregistered_command_key_propagates_to_children(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"[{ "key": "cmd+,", "command": "test.notRegistered" }]"#),
    );
    let probe = Probe::new(cx);
    let (wb, cx) = open(cx);
    wb.update(cx, |wb, cx| wb.set_editor_area(probe.clone().into(), cx));
    draw(cx);
    focus(&probe, cx);
    // Bound to a command nobody registered.
    cx.simulate_keystrokes("cmd-,");
    // Bound to a registered command: consumed by the workbench.
    cx.simulate_keystrokes("cmd-b");
    assert_eq!(
        probe.read_with(cx, |p, _| p.keys.clone()),
        vec![",".to_string()]
    );
}

#[gpui::test]
fn slots_show_views_and_track_focus(cx: &mut TestAppContext) {
    init(cx, None);
    let sidebar = Probe::new(cx);
    let editor = Probe::new(cx);
    let left = Probe::new(cx);
    let right = Probe::new(cx);
    let (wb, cx) = open(cx);
    // Hidden, the sidebar's slot is not drawn.
    cx.simulate_keystrokes("cmd-b");
    wb.update(cx, |wb, cx| {
        wb.set_sidebar_view(SidebarView::Explorer, sidebar.clone().into(), cx);
        wb.set_editor_area(editor.clone().into(), cx);
        wb.add_status_item(StatusBarSide::Left, left.clone().into(), cx);
        wb.add_status_item(StatusBarSide::Right, right.clone().into(), cx);
    });
    wb.read_with(cx, |wb, _| {
        assert!(same(wb.sidebar_view(SidebarView::Explorer), &sidebar));
        assert!(same(wb.editor_area(), &editor));
        // The built-in diagnostics counts, configuration error item and
        // vim mode indicator come first.
        let [_, _, _, l] = wb.status_items(StatusBarSide::Left) else {
            panic!("one left item after the built-in ones")
        };
        // The built-in large-file notice, language server status and
        // document status come first.
        let [_, _, _, r] = wb.status_items(StatusBarSide::Right) else {
            panic!("one right item after the built-in ones")
        };
        assert_eq!(l.entity_id(), left.entity_id());
        assert_eq!(r.entity_id(), right.entity_id());
    });

    draw(cx);
    assert_eq!(sidebar.read_with(cx, |p, _| p.renders), 0);
    for probe in [&editor, &left, &right] {
        assert!(probe.read_with(cx, |p, _| p.renders) > 0);
    }

    wb.update_in(cx, |wb, window, cx| {
        wb.show_sidebar_view(SidebarView::Explorer, window, cx)
    });
    draw(cx);
    assert!(sidebar.read_with(cx, |p, _| p.renders) > 0);

    focus(&sidebar, cx);
    let ctx = context(&wb, cx);
    assert!(holds(
        &ctx,
        "sideBarFocus && explorerViewletFocus && !searchViewletFocus && !editorFocus"
    ));

    focus(&editor, cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "editorFocus && !sideBarFocus"));
}

/// ⌘Q quits: the binding names `workbench.action.quit` and the command is
/// registered. (Quitting itself is not exercised.)
#[gpui::test]
fn cmd_q_is_bound_to_the_registered_quit_command(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = open(cx);
    let ctx = context(&wb, cx);
    let resolution = cx.update(|_, cx| {
        let keys = lucerna_settings::parse_keys("cmd+q").unwrap();
        cx.global::<AppState>().keymap.resolve(&keys, &ctx)
    });
    assert_eq!(
        resolution,
        lucerna_settings::Resolution::Command {
            command: "workbench.action.quit".into(),
            args: None
        }
    );
    let registered = wb.read_with(cx, |wb, _| {
        wb.commands()
            .all()
            .any(|(id, title)| id == "workbench.action.quit" && !title.is_empty())
    });
    assert!(registered);
}
