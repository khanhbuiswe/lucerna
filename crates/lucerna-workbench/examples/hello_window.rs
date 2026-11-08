//! Opens the workbench shell on the current directory.
//!
//! `cargo run -p lucerna-workbench --example hello_window -- "One Light"`
//! picks a theme by name (default "One Dark").
use gpui::{App, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use gpui_platform::application;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, Workbench};

fn main() {
    let theme_name = std::env::args().nth(1).unwrap_or_else(|| "One Dark".into());
    let root = std::env::current_dir().expect("current directory");
    application().run(move |cx: &mut App| {
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
        });
        let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workbench::new(root, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
