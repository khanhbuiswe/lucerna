//! Starts the app for `luc`: the configuration, themes and keymap, then
//! one workbench window on a folder, asking for the folder when the app
//! bundle is started from Finder or the Dock.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, PathPromptOptions, actions, px, size,
};
use gpui_platform::application;
use lucerna_extension::store::Store;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::{Associations, LanguageRegistry};
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};

use crate::extensions::{
    ExtensionService, WasmGrammarValidator, extension_languages, extensions_root,
};
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
        start(cx);
        open_workbench(launch, cx);
    });
}

/// Runs the app after asking for a folder to open, as when Finder or the
/// Dock starts the bundle with no path. Quits if the user cancels.
pub fn run_picking_folder(on_first_frame: Option<Box<dyn FnOnce()>>) {
    application().run(move |cx: &mut App| {
        start(cx);
        cx.activate(true);
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |cx| {
            let root = match picked.await {
                Ok(Ok(Some(paths))) => paths
                    .into_iter()
                    .next()
                    .and_then(|path| std::fs::canonicalize(path).ok()),
                Ok(Err(err)) => {
                    tracing::error!(%err, "could not show the folder picker");
                    None
                }
                _ => None,
            };
            cx.update(|cx| match root {
                Some(root) => open_workbench(
                    Launch {
                        root,
                        file: None,
                        single_file: false,
                        on_first_frame,
                    },
                    cx,
                ),
                None => cx.quit(),
            });
        })
        .detach();
    });
}

/// What every launch sets up before its first window.
fn start(cx: &mut App) {
    crate::fonts::register(cx);
    set_app_state(extensions_root(), cx);
    install_app_menu(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Opens the workbench window on `launch`; quits if it cannot.
fn open_workbench(launch: Launch, cx: &mut App) {
    let Launch {
        root,
        file,
        single_file,
        on_first_frame,
    } = launch;
    watch_config(ConfigPaths::for_root(&root), cx);
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
        // Next-frame callbacks run before that frame draws, so the
        // second one runs once the first frame has been presented.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                ExtensionService::schedule_startup_refresh(cx);
                if let Some(callback) = on_first_frame {
                    callback();
                }
            });
        });
        workbench
    });
    if let Err(err) = opened {
        tracing::error!(%err, "could not open the window");
        cx.quit();
        return;
    }
    cx.activate(true);
}

actions!(lucerna, [Quit, Hide, Minimize]);

/// The macOS app menu: Lucerna → Hide (⌘H), Quit (⌘Q), and Window →
/// Minimize (⌘M). The menu shows the shortcuts GPUI's keymap binds to its
/// actions; ⌘Q also resolves through the user keymap to
/// `workbench.action.quit`. Both quit the same way, so the layout is saved
/// by the workbench's `on_app_quit` hook.
fn install_app_menu(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| window.minimize_window())
                .ok();
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    cx.set_menus([
        Menu::new("Lucerna").items([
            MenuItem::action("Hide Lucerna", Hide),
            MenuItem::separator(),
            MenuItem::action("Quit Lucerna", Quit),
        ]),
        Menu::new("Window").items([MenuItem::action("Minimize", Minimize)]),
    ]);
}

/// The built-in defaults plus the installed extensions, read from the
/// store's `index.json` without loading any grammar; [`watch_config`] then
/// applies the user's files. Starts the [`ExtensionService`] on the store
/// at `extensions_root`. Only `index.json` is read here (spec §8); the
/// store's leftover sweep and index check run in the background.
pub fn set_app_state(extensions_root: PathBuf, cx: &mut App) {
    let store = Store::open_deferred(extensions_root, Arc::new(WasmGrammarValidator));
    set_app_state_with_store(store, cx);
}

/// [`set_app_state`] on a given store (tests use one with a fake registry
/// and toolchain).
pub fn set_app_state_with_store(store: Store, cx: &mut App) {
    let languages = LanguageRegistry::build(
        &extension_languages(&store.index()),
        Associations::default(),
    );
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    let (icon_themes, _) = IconThemeRegistry::build(&[]);
    let icons = icon_themes.resolve("Lucide");
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
        languages: Arc::new(languages),
        languages_generation: 0,
        icon_themes,
        icons,
    });
    ExtensionService::start(store, cx);
}
