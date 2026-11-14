//! Opens the workbench shell on the current directory.
//!
//! `cargo run -p lucerna-workbench --example hello_window -- "Lucerna Light"`
//! picks a theme by name (default "Lucerna Dark").
use std::sync::Arc;

use gpui::{App, Bounds, prelude::*, px, size};
use gpui_platform::application;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, Workbench, window_options};

fn main() {
    let theme_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Lucerna Dark".into());
    let root = std::env::current_dir().expect("current directory");
    application().run(move |cx: &mut App| {
        lucerna_workbench::fonts::register(cx);
        let themes = ThemeRegistry::builtin();
        let theme = themes.get_or_default(&theme_name);
        let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
        if let Some(error) = error {
            eprintln!("keymap: {}", error.message);
        }
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
        let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
        cx.open_window(window_options(bounds), |window, cx| {
            cx.new(|cx| Workbench::new(root, window, cx))
        })
        .unwrap();
        cx.activate(true);
    });
}
