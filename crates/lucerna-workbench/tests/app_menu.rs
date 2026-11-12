//! The menu bar's actions, driven through GPUI's test platform.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::app_menu::{self, RunCommand};
use lucerna_workbench::{AppState, Workbench};

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
