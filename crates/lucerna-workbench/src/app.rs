//! Starts the app for `luc`: the configuration, themes and keymap, then
//! one workbench window on a folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, AppContext as _, Bounds, px, size};
use gpui_platform::application;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;

use crate::{
    AppState, ConfigPaths, OpenMode, Workbench, default_data_dir, watch_config, window_options,
};

/// What [`run`] opens.
pub struct Launch {
    /// The folder to show, already canonical: the workspace state file and
    /// tab paths are keyed by it.
    pub root: PathBuf,
    /// A file to open as a pinned tab.
    pub file: Option<PathBuf>,
    /// Open `file` alone: `root` is not scanned or watched (R27).
    pub single_file: bool,
    /// Called once the first frame of the window is painted.
    pub on_first_frame: Option<Box<dyn FnOnce()>>,
}

/// Runs the app until it quits; it quits when its last window closes.
pub fn run(launch: Launch) {
    application().run(move |cx: &mut App| {
        let Launch {
            root,
            file,
            single_file,
            on_first_frame,
        } = launch;
        set_app_state(cx);
        watch_config(ConfigPaths::for_root(&root), cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
        let opened = cx.open_window(window_options(bounds), |window, cx| {
            let workbench = cx.new(|cx| match (&file, default_data_dir()) {
                (Some(file), data_dir) if single_file => {
                    Workbench::single_file(root, file, data_dir, window, cx)
                }
                (_, Some(data_dir)) => Workbench::with_data_dir(root, data_dir, window, cx),
                (_, None) => Workbench::new(root, window, cx),
            });
            if let Some(file) = file {
                let path: Arc<Path> = file.into();
                workbench.update(cx, |wb, cx| wb.open_path(path, OpenMode::Pinned, cx));
            }
            if let Some(callback) = on_first_frame {
                // Next-frame callbacks run before that frame draws, so the
                // second one runs once the first frame has been presented.
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |_, _| callback());
                });
            }
            workbench
        });
        if let Err(err) = opened {
            tracing::error!(%err, "could not open the window");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}

/// The built-in defaults; [`watch_config`] then applies the user's files.
fn set_app_state(cx: &mut App) {
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    if let Some(error) = error {
        tracing::error!(message = error.message, "default keymap");
    }
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
    });
}
