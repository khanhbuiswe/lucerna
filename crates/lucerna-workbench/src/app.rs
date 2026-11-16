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
use gpui::{App, AppContext as _, AsyncApp, Global, PathPromptOptions, QuitMode};
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
use crate::recent::{self, RecentStore};
pub use crate::router::Launch;
use crate::router::{self, OpenRequest, Source, Target, app_data_dir};
use crate::session::{self, SessionWindow, session_path};
use crate::startup::{RestoreWindows, Startup, StartupScreen, startup_screen};
use crate::window_choice::file_paths;
use crate::{
    AppState, ConfigPaths, default_data_dir, last_workspace, restore_windows, startup_choice,
    watch_config,
};

/// How long a launch without a path waits for the platform to ask for a
/// path (Finder's "Open With") before it shows the folder picker.
const OPEN_REQUEST_GRACE: Duration = Duration::from_millis(300);

/// The socket `luc` claimed for this app ([`crate::cli_socket`]); none on
/// platforms without Unix sockets yet.
#[cfg(unix)]
pub type CliListener = crate::cli_socket::Bound;
/// The socket `luc` claimed for this app; none on platforms without Unix
/// sockets yet.
#[cfg(not(unix))]
pub type CliListener = std::convert::Infallible;

/// Runs the app until it quits. Closing the last window quits on Linux
/// and Windows; on macOS the app keeps running with its menu bar, and
/// clicking its Dock icon then opens the Welcome screen (spec M8, M9).
///
/// With `launch`, a window opens on it, after the last session's windows
/// when `window.restoreWindows` is `"preserve"`. Without, as when Finder
/// or the Dock starts the app bundle, the app waits briefly for a path to
/// open and otherwise reopens the last session's windows (spec §6.6),
/// shows the Get Started page on the first run, or the Welcome screen, or
/// asks for a folder when `workbench.startupEditor` is `"none"` (Cancel
/// quits). Paths the platform asks to open later go through
/// [`router::open`].
///
/// With `listener`, the socket `luc` bound, requests from later `luc`
/// commands are read on a thread from the start and answered once the
/// first windows are up (spec §6.5); quitting removes the socket file.
pub fn run(launch: Option<Launch>, listener: Option<CliListener>) {
    let cli = listener.map(serve_cli);
    let (requests_tx, mut requests) = mpsc::unbounded::<Vec<String>>();
    let app = application().with_quit_mode(QuitMode::Default);
    app.on_open_urls(move |urls| {
        requests_tx.unbounded_send(urls).ok();
    });
    app.on_reopen(on_reopen);
    app.run(move |cx: &mut App| {
        start(cx);
        let cli = cli.map(|cli| cli.remove_on_quit(cx));
        // Once for the app: each window reads its own folder's settings.
        watch_config(ConfigPaths::user_only(), cx);
        let waiting = launch.is_none();
        if let Some(launch) = launch {
            open_launch_at_start(launch, cx);
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
                    Either::Left((None, _)) => {}
                    Either::Right(_) => open_without_folder(cx).await,
                }
            }
            // Only now: a request during the wait above would race the
            // first screen; one during a restore waits in the router.
            if let Some(cli) = cli {
                cli.answer(cx);
            }
            while let Some(urls) = requests.next().await {
                cx.update(|cx| open_urls(&urls, cx));
            }
        })
        .detach();
    });
}

/// The socket's requests on their way to the main thread.
struct CliRequests {
    #[cfg(unix)]
    requests: mpsc::UnboundedReceiver<crate::cli_socket::CliMessage>,
    #[cfg(unix)]
    socket: Option<crate::cli_socket::SocketFile>,
}

/// Serves `listener` on a thread from now on ([`cli_socket::serve`]).
///
/// [`cli_socket::serve`]: crate::cli_socket::serve
#[cfg(unix)]
fn serve_cli(bound: CliListener) -> CliRequests {
    let (tx, requests) = mpsc::unbounded();
    crate::cli_socket::serve(bound.listener, tx);
    CliRequests {
        requests,
        socket: Some(bound.file),
    }
}

#[cfg(not(unix))]
fn serve_cli(listener: CliListener) -> CliRequests {
    match listener {}
}

impl CliRequests {
    /// Removes the socket file when the app quits, so the next `luc`
    /// binds at once; not when another app's socket replaced it
    /// ([`SocketFile::remove`]).
    ///
    /// [`SocketFile::remove`]: crate::cli_socket::SocketFile::remove
    fn remove_on_quit(mut self, cx: &mut App) -> CliRequests {
        #[cfg(unix)]
        if let Some(socket) = self.socket.take() {
            cx.on_app_quit(move |_| {
                let path = socket.path();
                match socket.remove() {
                    Ok(true) => {}
                    Ok(false) => tracing::info!(path = %path.display(), "the luc socket is no longer ours"),
                    Err(err) => {
                        tracing::warn!(%err, path = %path.display(), "could not remove the luc socket");
                    }
                }
                async {}
            })
            .detach();
        }
        let _ = cx;
        self
    }

    /// Answers each request on the main thread ([`router::answer_cli`]).
    fn answer(self, cx: &mut AsyncApp) {
        #[cfg(unix)]
        {
            let mut requests = self.requests;
            cx.spawn(async move |cx| {
                while let Some((request, reply)) = requests.next().await {
                    tracing::info!(paths = ?request.paths, "luc asks to open");
                    cx.update(|cx| router::answer_cli(request, reply, cx));
                }
            })
            .detach();
        }
        #[cfg(not(unix))]
        let _ = (self, cx);
    }
}

/// Opens `launch`, the path `luc` was started with. With
/// `window.restoreWindows: "preserve"` the last session's windows reopen
/// first (read in the background); otherwise only `launch` opens, as in
/// VS Code.
fn open_launch_at_start(launch: Launch, cx: &mut App) {
    let preserve = restore_windows(&cx.global::<AppState>().settings) == RestoreWindows::Preserve;
    let data_dir = app_data_dir(cx);
    let Some(data_dir) = data_dir.filter(|_| preserve) else {
        router::open_launch(launch, Target::Auto, cx);
        quit_if_no_window("the first window", cx);
        return;
    };
    router::begin_restore(cx);
    cx.spawn(async move |cx| {
        let windows = cx
            .background_spawn(async move {
                let saved = session::load(&session_path(&data_dir));
                session::to_restore(&saved, RestoreWindows::Preserve, Path::exists)
            })
            .await;
        cx.update(|cx| {
            router::restore(windows, cx);
            router::open_launch(launch, Target::Auto, cx);
            quit_if_no_window("the first window", cx);
        });
    })
    .detach();
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

/// The Dock icon was clicked with no window shown (spec M9): the Welcome
/// screen in a new window, as VS Code opens an empty one. With windows
/// that are only minimized, the most recently used comes forward.
fn on_reopen(cx: &mut App) {
    router::reopen(cx);
}

/// [`on_reopen`], which the test platform cannot call.
#[doc(hidden)]
pub fn on_reopen_for_test(cx: &mut App) {
    on_reopen(cx);
}

/// Counts quits in place of quitting; set by [`count_quits_for_test`].
struct QuitCount(usize);

impl Global for QuitCount {}

/// From now on [`quit_now`] counts instead of quitting: the test
/// platform's `quit` does nothing.
#[doc(hidden)]
pub fn count_quits_for_test(cx: &mut App) {
    cx.set_global(QuitCount(0));
}

/// How many times the app would have quit since [`count_quits_for_test`].
#[doc(hidden)]
pub fn quits_for_test(cx: &App) -> usize {
    cx.try_global::<QuitCount>().map_or(0, |count| count.0)
}

/// Quits the app: `cx.quit()`, or a count in tests.
pub(crate) fn quit_now(cx: &mut App) {
    if cx.has_global::<QuitCount>() {
        cx.global_mut::<QuitCount>().0 += 1;
    } else {
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

/// With no folder to open: opens what [`screen_for`] picks, its files
/// read in the background. Requests from `luc` and the platform wait
/// until then (spec M14).
async fn open_without_folder(cx: &mut AsyncApp) {
    let (data_dir, mode, choice, hot_exit) = cx.update(|cx| {
        router::begin_restore(cx);
        let settings = &cx.global::<AppState>().settings;
        (
            app_data_dir(cx),
            restore_windows(settings),
            startup_choice(settings),
            hot_exit_on(settings),
        )
    });
    let dir = data_dir.clone();
    let screen = cx
        .background_spawn(async move { screen_for(dir.as_deref(), mode, choice, hot_exit) })
        .await;
    cx.update(|cx| open_screen(screen, data_dir, cx));
}

/// Opens `screen`, then the requests that waited for it. The Get Started
/// page is remembered as shown as soon as it opens (spec O3).
fn open_screen(screen: StartupScreen, data_dir: Option<PathBuf>, cx: &mut App) {
    match screen {
        StartupScreen::Restore(windows) => {
            tracing::info!(
                count = windows.len(),
                "reopening the last session's windows"
            );
            router::restore(windows, cx);
            quit_if_no_window("the last session's windows", cx);
            return;
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
    router::finish_restore(cx);
}

/// [`screen_for`] with `settings` read as a start reads them.
#[cfg(test)]
pub(crate) fn first_screen(data_dir: Option<&Path>, settings: &SettingsStore) -> StartupScreen {
    screen_for(
        data_dir,
        restore_windows(settings),
        startup_choice(settings),
        hot_exit_on(settings),
    )
}

/// `files.hotExit` is not `"off"`.
fn hot_exit_on(settings: &SettingsStore) -> bool {
    settings.get::<String>("files.hotExit", None).as_deref() != Some("off")
}

/// What a start without a path shows, from the app data directory and the
/// user settings (`mode`, `choice`): the last session's windows that
/// `window.restoreWindows` keeps, else (unless it is `"none"`) the newest
/// recent entry that still exists, then the Get Started page on the first run, then what
/// `workbench.startupEditor` says. Without a data directory there is no
/// first run to remember, so no Get Started page. With `hot_exit` on, the
/// single-file windows that left backups join the list even when the
/// restore mode keeps none (spec §5.4). Reads `windows.json`, the recent
/// list, `state.json` and the backups.
fn screen_for(
    data_dir: Option<&Path>,
    mode: RestoreWindows,
    choice: Startup,
    hot_exit: bool,
) -> StartupScreen {
    let mut restore = match data_dir {
        Some(dir) if mode != RestoreWindows::None => {
            let saved = session::load(&session_path(dir));
            let windows = session::to_restore(&saved, mode, Path::exists);
            if windows.is_empty() {
                let recent = recent::load(&recent::recent_path(dir));
                last_workspace(&recent)
                    .map(|entry| SessionWindow {
                        kind: entry.kind,
                        path: entry.path.clone(),
                    })
                    .into_iter()
                    .collect()
            } else {
                windows
            }
        }
        _ => Vec::new(),
    };
    if let (Some(dir), true) = (data_dir, hot_exit) {
        add_backed_up_files(&mut restore, dir);
    }
    // The flag is not needed when windows are reopened.
    let onboarded = match restore.is_empty() {
        true => data_dir.map(onboarded),
        false => None,
    };
    startup_screen(restore, onboarded, choice)
}

/// Adds a file window for each file with backups that is not already in
/// `windows` (compared by canonical path) and still exists.
fn add_backed_up_files(windows: &mut Vec<SessionWindow>, data_dir: &Path) {
    let mut files = crate::hot_exit::store::file_windows_with_backups(data_dir);
    files.sort();
    let mut seen: Vec<PathBuf> = windows
        .iter()
        .filter_map(|window| window.path.canonicalize().ok())
        .collect();
    for file in files {
        let Ok(canonical) = file.canonicalize() else {
            continue;
        };
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical);
        windows.push(SessionWindow {
            kind: crate::recent::RecentKind::File,
            path: file,
        });
    }
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
    // Closing the last window is left to `QuitMode::Default` (spec M8).
    app_menu::install(cx);
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
    use crate::recent::RecentKind;
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

    fn folder_window(path: PathBuf) -> SessionWindow {
        SessionWindow {
            kind: RecentKind::Folder,
            path,
        }
    }

    /// Writes `windows.json` in `dir` listing `windows`.
    fn write_session(dir: &Path, windows: Vec<SessionWindow>) {
        session::save(&session_path(dir), &session::Session { windows }).unwrap();
    }

    fn expected_window(screen: &StartupScreen) -> SessionWindow {
        match screen {
            StartupScreen::Restore(windows) => windows[0].clone(),
            other => panic!("not a restore: {other:?}"),
        }
    }

    #[test]
    fn file_window_with_backup_reopens() {
        use crate::hot_exit::store::{
            Backup, BackupHeader, WorkspaceKey, backup_dir, text_sha256, write_backup,
        };
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "base").unwrap();
        let key = WorkspaceKey::File(file.clone());
        let backup = Backup {
            header: BackupHeader {
                path: file.clone(),
                mtime: None,
                size: None,
                base_sha256: Some(text_sha256("base")),
                encoding: "UTF-8".into(),
                bom: false,
                line_ending: lucerna_text::LineEnding::Lf,
            },
            text: "edited".into(),
        };
        write_backup(&backup_dir(dir.path(), &key), &key, &backup).unwrap();
        let expected = StartupScreen::Restore(vec![SessionWindow {
            kind: RecentKind::File,
            path: file,
        }]);
        let none = with_user(r#"{"window.restoreWindows":"none"}"#);
        assert_eq!(first_screen(Some(dir.path()), &none), expected);
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            expected
        );
        write_session(dir.path(), vec![expected_window(&expected)]);
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            expected,
            "listed once when the session has it too"
        );
        write_session(dir.path(), Vec::new());
        let off = with_user(r#"{"files.hotExit":"off"}"#);
        assert_ne!(first_screen(Some(dir.path()), &off), expected);
    }

    #[test]
    fn first_screen_restores_existing_folder() {
        let (dir, folder) = with_recent_folder();
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            StartupScreen::Restore(vec![folder_window(folder)])
        );
    }

    #[test]
    fn first_screen_restores_all_windows() {
        let (dir, recent_folder) = with_recent_folder();
        let other = dir.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "").unwrap();
        let file_window = SessionWindow {
            kind: RecentKind::File,
            path: file,
        };
        write_session(
            dir.path(),
            vec![
                folder_window(other.clone()),
                folder_window(dir.path().join("gone")),
                file_window.clone(),
                folder_window(recent_folder.clone()),
            ],
        );
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            StartupScreen::Restore(vec![
                folder_window(other.clone()),
                file_window,
                folder_window(recent_folder.clone()),
            ])
        );
        let store = with_user(r#"{"window.restoreWindows":"folders"}"#);
        assert_eq!(
            first_screen(Some(dir.path()), &store),
            StartupScreen::Restore(vec![folder_window(other), folder_window(recent_folder)])
        );
    }

    #[test]
    fn first_screen_empty_session_falls_back_to_recent() {
        let (dir, folder) = with_recent_folder();
        write_session(dir.path(), Vec::new());
        assert_eq!(
            first_screen(Some(dir.path()), &SettingsStore::with_defaults()),
            StartupScreen::Restore(vec![folder_window(folder.clone())])
        );
        // Every window of the session is gone.
        write_session(dir.path(), vec![folder_window(dir.path().join("gone"))]);
        let store = with_user(r#"{"window.restoreWindows":"one"}"#);
        assert_eq!(
            first_screen(Some(dir.path()), &store),
            StartupScreen::Restore(vec![folder_window(folder)])
        );
    }

    #[test]
    fn first_screen_none_ignores_session() {
        let (dir, folder) = with_recent_folder();
        write_session(dir.path(), vec![folder_window(folder)]);
        let store = with_user(r#"{"window.restoreWindows":"none"}"#);
        assert_eq!(
            first_screen(Some(dir.path()), &store),
            StartupScreen::Onboarding
        );
        crate::app_state_file::mark_onboarded(dir.path()).unwrap();
        assert_eq!(
            first_screen(Some(dir.path()), &store),
            StartupScreen::Welcome
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
