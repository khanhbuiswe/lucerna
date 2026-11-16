//! Starts the app for `luc`: the configuration, themes and keymap, then
//! a workbench window on a folder, or the Welcome screen (the Get Started
//! page on the first run) when the app bundle is started from Finder or
//! the Dock. Every window, and every path the platform sends later, goes
//! through the [`router`].

use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::future::{self, Either};
use gpui::{App, AppContext as _, PathPromptOptions};
use gpui_platform::application;
use lucerna_extension::store::Store;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::{Associations, LanguageRegistry};
use lucerna_theme::{DEFAULT_ICON_THEME, IconThemeRegistry, ThemeRegistry};

use crate::app_menu;
use crate::app_state_file::{mark_onboarded, onboarded};
use crate::extensions::{
    ExtensionService, WasmGrammarValidator, extension_languages, extensions_root,
};
use crate::recent::{self, RecentList, RecentStore};
pub use crate::router::Launch;
use crate::router::{self, OpenRequest, Source, Target, app_data_dir};
use crate::startup::{StartupScreen, startup_screen};
use crate::window_choice::file_paths;
use crate::{
    AppState, ConfigPaths, default_data_dir, last_workspace, restores_last_workspace,
    startup_choice, watch_config,
};

/// How long a launch without a path waits for the platform to ask for a
/// path (Finder's "Open With") before it shows the folder picker.
const OPEN_REQUEST_GRACE: Duration = Duration::from_millis(300);

/// Runs the app until it quits; it quits when its last window closes.
///
/// With `launch`, a window opens on it. Without, as when Finder or the
/// Dock starts the app bundle, the app waits briefly for a path to open and
/// otherwise reopens the last workspace, shows the Get Started page on the
/// first run, or the Welcome screen, or asks for a folder when
/// `workbench.startupEditor` is `"none"` (Cancel quits). Paths the platform
/// asks to open later go through [`router::open`].
pub fn run(launch: Option<Launch>) {
    let (requests_tx, mut requests) = mpsc::unbounded::<Vec<String>>();
    let app = application();
    app.on_open_urls(move |urls| {
        requests_tx.unbounded_send(urls).ok();
    });
    app.run(move |cx: &mut App| {
        start(cx);
        // Once for the app: each window reads its own folder's settings.
        watch_config(ConfigPaths::user_only(), cx);
        let waiting = launch.is_none();
        if let Some(launch) = launch {
            router::open_launch(launch, Target::Auto, cx);
            quit_if_no_window("the first window", cx);
        } else {
            cx.activate(true);
        }
        cx.spawn(async move |cx| {
            if waiting {
                let grace = cx.background_executor().timer(OPEN_REQUEST_GRACE);
                match future::select(requests.next(), pin!(grace)).await {
                    Either::Left((Some(urls), _)) => cx.update(|cx| {
                        open_urls(&urls, cx);
                        quit_if_no_window("the requested paths", cx);
                    }),
                    Either::Left((None, _)) => return,
                    Either::Right(_) => cx.update(open_without_folder),
                }
            }
            while let Some(urls) = requests.next().await {
                cx.update(|cx| open_urls(&urls, cx));
            }
        })
        .detach();
    });
}

/// Whether a start must quit after trying to open its first window: no
/// window opened, so nothing would ever show (opening a window can fail).
/// Later opens, with windows already up, never quit the app.
pub(crate) fn start_failed(windows: usize) -> bool {
    windows == 0
}

/// Quits when the start opened no window ([`start_failed`]); `what` names
/// what it tried to open, for the log.
fn quit_if_no_window(what: &str, cx: &mut App) {
    if start_failed(cx.windows().len()) {
        tracing::error!(what, "nothing opened at start; quitting");
        cx.quit();
    }
}

/// Opens the paths the platform asks for (Finder's "Open With", the Dock).
fn open_urls(urls: &[String], cx: &mut App) {
    let request = OpenRequest {
        paths: file_paths(urls),
        target: Target::Auto,
        source: Source::Finder,
    };
    router::open(request, cx).ok();
}

/// With no folder to open: opens what [`first_screen`] picks. The Get
/// Started page is remembered as shown as soon as it opens (spec O3).
fn open_without_folder(cx: &mut App) {
    let data_dir = app_data_dir(cx);
    match first_screen(data_dir.as_deref(), &cx.global::<AppState>().settings) {
        StartupScreen::Restore(path) => {
            tracing::info!(path = %path.display(), "reopening the last workspace");
            let request = OpenRequest {
                paths: vec![path],
                target: Target::Auto,
                source: Source::Restore,
            };
            router::open(request, cx).ok();
            quit_if_no_window("the last workspace", cx);
        }
        StartupScreen::Onboarding => {
            if let Some(dir) = data_dir {
                cx.background_spawn(async move {
                    if let Err(err) = mark_onboarded(&dir) {
                        tracing::warn!(%err, "could not remember the onboarding as shown");
                    }
                })
                .detach();
            }
            router::open_onboarding(cx);
            quit_if_no_window("the Get Started page", cx);
        }
        StartupScreen::Welcome => {
            router::open_welcome(Target::NewWindow, cx);
            quit_if_no_window("the Welcome screen", cx);
        }
        StartupScreen::FolderPrompt => pick_folder(cx),
    }
}

/// What a start without a path shows, from the app data directory and the
/// user settings: the last workspace when `window.restoreWindows` allows
/// it, then the Get Started page on the first run, then what
/// `workbench.startupEditor` says. Without a data directory there is no
/// first run to remember, so no Get Started page. Reads the recent list
/// and `state.json` on this thread.
pub(crate) fn first_screen(data_dir: Option<&Path>, settings: &SettingsStore) -> StartupScreen {
    let recent = match data_dir {
        Some(dir) if restores_last_workspace(settings) => recent::load(&recent::recent_path(dir)),
        _ => RecentList::default(),
    };
    let restore = last_workspace(&recent);
    // The flag is not needed when a workspace is reopened.
    let onboarded = match restore {
        Some(_) => None,
        None => data_dir.map(onboarded),
    };
    startup_screen(restore, onboarded, startup_choice(settings))
}

/// Asks for a folder to open; quits on Cancel unless a window opened
/// meanwhile.
fn pick_folder(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open".into()),
    });
    cx.spawn(async move |cx| {
        let folder = match picked.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            Ok(Err(err)) => {
                tracing::error!(%err, "could not show the folder picker");
                None
            }
            _ => None,
        };
        cx.update(|cx| match folder {
            Some(folder) => {
                let request = OpenRequest {
                    paths: vec![folder],
                    target: Target::Auto,
                    source: Source::Menu,
                };
                router::open(request, cx).ok();
            }
            None if cx.windows().is_empty() => cx.quit(),
            None => {}
        });
    })
    .detach();
}

/// What every launch sets up before its first window.
fn start(cx: &mut App) {
    crate::fonts::register(cx);
    set_app_state(extensions_root(), cx);
    cx.set_global(RecentStore {
        data_dir: default_data_dir(),
    });
    app_menu::install(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
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
    let theme = themes.get_or_default("Lucerna Dark");
    let (icon_themes, _) = IconThemeRegistry::build(&[]);
    let icons = icon_themes.resolve(DEFAULT_ICON_THEME);
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

#[cfg(test)]
mod tests {
    use lucerna_settings::Layer;

    use super::*;
    use crate::startup::StartupScreen;

    /// A data dir whose `recent.json` lists one folder that exists.
    fn with_recent_folder() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let file = serde_json::json!({
            "version": 1,
            "entries": [{ "path": folder, "kind": "folder" }],
        });
        std::fs::write(recent::recent_path(dir.path()), file.to_string()).unwrap();
        (dir, folder)
    }

    fn with_user(json: &str) -> SettingsStore {
        let mut store = SettingsStore::with_defaults();
        store.set_layer(Layer::User, json).unwrap();
        store
    }

    #[test]
    fn start_quits_only_without_a_window() {
        assert!(start_failed(0), "a failed first window quits");
        assert!(!start_failed(1));
        assert!(!start_failed(3));
    }

    #[test]
    fn first_screen_fresh_is_onboarding() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            StartupScreen::Onboarding
        );
    }

    #[test]
    fn first_screen_restores_existing_folder() {
        let (dir, folder) = with_recent_folder();
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            StartupScreen::Restore(folder)
        );
    }

    #[test]
    fn first_screen_after_onboarded_is_welcome() {
        let dir = tempfile::tempdir().unwrap();
        crate::app_state_file::mark_onboarded(dir.path()).unwrap();
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            StartupScreen::Welcome
        );
        let store = with_user(r#"{"workbench.startupEditor":"none"}"#);
        assert_eq!(
            first_screen(Some(dir.path()), &store),
            StartupScreen::FolderPrompt
        );
    }

    #[test]
    fn first_screen_restore_none_first_run() {
        let (dir, _) = with_recent_folder();
        let store = with_user(r#"{"window.restoreWindows":"none"}"#);
        assert_eq!(
            first_screen(Some(dir.path()), &store),
            StartupScreen::Onboarding
        );
    }

    #[test]
    fn first_screen_no_data_dir() {
        assert_eq!(
            first_screen(None, &SettingsStore::with_defaults()),
            StartupScreen::Welcome
        );
    }
}
