//! The menu bar's actions, driven through GPUI's test platform.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::app_menu::{self, DisabledCommand, RunCommand};
use lucerna_workbench::recent::RecentStore;
use lucerna_workbench::{AppState, WelcomeView, Workbench};

const TOGGLE_SIDEBAR: &str = "workbench.action.toggleSidebarVisibility";

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
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
    cx.update(app_menu::install);
}

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx))
}

fn sidebar_visible(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    wb.read_with(cx, |wb, _| wb.sidebar_visible())
}

#[gpui::test]
fn run_command_reaches_the_workbench(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    assert!(sidebar_visible(&wb, cx));
    cx.dispatch_action(RunCommand {
        id: TOGGLE_SIDEBAR.into(),
    });
    cx.run_until_parked();
    assert!(!sidebar_visible(&wb, cx));
}

#[gpui::test]
fn menu_binding_does_not_double_run(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    // The menu shows ⌘B through a GPUI binding of its own.
    let bound = cx.update(|_, cx| {
        let action = RunCommand {
            id: TOGGLE_SIDEBAR.into(),
        };
        cx.key_bindings()
            .borrow()
            .bindings_for_action(&action)
            .count()
    });
    assert_eq!(bound, 1);
    cx.simulate_keystrokes("cmd-b");
    cx.run_until_parked();
    assert!(
        !sidebar_visible(&wb, cx),
        "cmd-b ran an even number of times"
    );
}

/// AppKit enables a menu item when GPUI says its action is available, so
/// what a Welcome window cannot run uses an action nothing handles.
#[gpui::test]
fn welcome_window_cannot_run_workbench_items(cx: &mut TestAppContext) {
    init(cx);
    cx.set_global(RecentStore { data_dir: None });
    let (_, cx) = cx.add_window_view(WelcomeView::new);
    cx.run_until_parked();
    let (disabled, open_recent) = cx.update(|_, cx| {
        let save = DisabledCommand {
            id: "workbench.action.files.save".into(),
        };
        let open_recent = RunCommand {
            id: "workbench.action.openRecent".into(),
        };
        (
            cx.is_action_available(&save),
            cx.is_action_available(&open_recent),
        )
    });
    assert!(!disabled);
    assert!(open_recent);
}
