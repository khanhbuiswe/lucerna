//! Starts the app for `luc`: the configuration, themes and keymap, then
//! one workbench window on a folder, or the Welcome screen when the app
//! bundle is started from Finder or the Dock, and opening the paths the
//! platform sends ([`open_requests`]).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::future::{self, Either};
use gpui::{
    App, AppContext as _, Bounds, Entity, Global, PathPromptOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, px, size,
};
use gpui_platform::application;
use lucerna_extension::store::Store;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::{Associations, LanguageRegistry};
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};

use crate::app_menu;
use crate::extensions::{
    ExtensionService, WasmGrammarValidator, extension_languages, extensions_root,
};
use crate::open_requests::{self, OpenWindow, Route};
use crate::process::{self, SpawnRequest, Spawner};
use crate::recent::{self, RecentEntry, RecentKind, RecentStore};
use crate::{
    AppState, ConfigPaths, OpenMode, Startup, WelcomeEvent, WelcomeView, Workbench,
    default_data_dir, startup_choice, watch_config, window_options,
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

/// Turns a path the platform asked to open into what [`run`] opens, as
/// `luc <path>` would.
pub type Resolve = Box<dyn Fn(PathBuf) -> Result<Launch, String>>;

/// How long a launch without a path waits for the platform to ask for a
/// path (Finder's "Open With") before it shows the folder picker.
const OPEN_REQUEST_GRACE: Duration = Duration::from_millis(300);

/// Runs the app until it quits; it quits when its last window closes.
///
/// With `launch`, the window opens on it. Without, as when Finder or the
/// Dock starts the app bundle, the app waits briefly for a path to open and
/// otherwise shows the Welcome screen, or asks for a folder when
/// `workbench.startupEditor` is `"none"` (Cancel quits). Paths the platform
/// asks to open later go through [`open_requests::route`], resolved by
/// `resolve`.
pub fn run(launch: Option<Launch>, resolve: Resolve) {
    let (requests_tx, mut requests) = mpsc::unbounded::<Vec<String>>();
    let app = application();
    app.on_open_urls(move |urls| {
        requests_tx.unbounded_send(urls).ok();
    });
    app.run(move |cx: &mut App| {
        start(cx);
        let router = Rc::new(RefCell::new(Router {
            shown: None,
            resolve,
            user_dir: ConfigPaths::user_only().user_dir,
        }));
        let waiting = launch.is_none();
        if let Some(launch) = launch {
            Router::open_first(&router, launch, cx);
        } else {
            cx.activate(true);
        }
        cx.spawn(async move |cx| {
            if waiting {
                let grace = cx.background_executor().timer(OPEN_REQUEST_GRACE);
                match future::select(requests.next(), pin!(grace)).await {
                    Either::Left((Some(urls), _)) => {
                        cx.update(|cx| Router::open_urls(&router, &urls, cx));
                    }
                    Either::Left((None, _)) => return,
                    Either::Right(_) => cx.update(|cx| open_without_folder(&router, cx)),
                }
            }
            while let Some(urls) = requests.next().await {
                cx.update(|cx| Router::open_urls(&router, &urls, cx));
            }
        })
        .detach();
    });
}

/// With no folder to open: reads the user's configuration, then shows the
/// Welcome screen or the folder prompt as `workbench.startupEditor` says.
fn open_without_folder(router: &Rc<RefCell<Router>>, cx: &mut App) {
    let user_dir = router.borrow().user_dir.clone();
    watch_config(ConfigPaths::user_only_with_user_dir(user_dir), cx);
    match startup_choice(&cx.global::<AppState>().settings) {
        Startup::FolderPrompt => pick_folder(router, cx),
        Startup::Welcome => Router::open_welcome(router, cx),
    }
}

/// What the process's one window shows.
enum Shown {
    Welcome(WindowHandle<WelcomeView>),
    Workbench(OpenWindow, WindowHandle<Workbench>),
}

/// The process's one window, and how to open paths the platform sends.
struct Router {
    shown: Option<Shown>,
    resolve: Resolve,
    /// The user configuration directory ([`ConfigPaths::user_dir`]).
    user_dir: PathBuf,
}

impl Router {
    /// Opens the workbench on `launch`: in place of the Welcome screen when
    /// it shows, else in a new window. Then records it as recently opened.
    fn open_first(this: &Rc<RefCell<Router>>, launch: Launch, cx: &mut App) {
        let (open, entry) = match (&launch.file, launch.single_file) {
            (Some(file), true) => (
                OpenWindow::single_file(file.clone()),
                RecentEntry {
                    path: file.clone(),
                    kind: RecentKind::File,
                },
            ),
            _ => (
                OpenWindow::folder(launch.root.clone()),
                RecentEntry {
                    path: launch.root.clone(),
                    kind: RecentKind::Folder,
                },
            ),
        };
        let (welcome, user_dir) = {
            let router = this.borrow();
            let welcome = match router.shown {
                Some(Shown::Welcome(handle)) => Some(handle),
                _ => None,
            };
            (welcome, router.user_dir.clone())
        };
        watch_config(ConfigPaths::with_user_dir(user_dir, &launch.root), cx);
        let handle = match welcome {
            Some(welcome) => replace_welcome(welcome, launch, cx),
            None => open_workbench(launch, cx),
        };
        if let Some(handle) = handle {
            this.borrow_mut().shown = Some(Shown::Workbench(open, handle));
            // The menu bar enables the workbench's items.
            app_menu::refresh(cx);
            record_recent(entry, cx);
        }
    }

    /// Opens the Welcome screen in a window like the workbench's; quits if
    /// it cannot.
    fn open_welcome(this: &Rc<RefCell<Router>>, cx: &mut App) {
        let router = this.clone();
        let opened = cx.open_window(maximized_window_options(cx), |window, cx| {
            let view = cx.new(|cx| WelcomeView::new(window, cx));
            cx.subscribe(&view, move |_, event: &WelcomeEvent, cx| {
                Router::on_welcome_event(&router, event, cx)
            })
            .detach();
            view
        });
        match opened {
            Ok(handle) => {
                cx.activate(true);
                this.borrow_mut().shown = Some(Shown::Welcome(handle));
            }
            Err(err) => {
                tracing::error!(%err, "could not open the window");
                cx.quit();
            }
        }
    }

    fn on_welcome_event(this: &Rc<RefCell<Router>>, event: &WelcomeEvent, cx: &mut App) {
        match event {
            WelcomeEvent::Open { path, new_window } => {
                Router::open_from_welcome(this, path.clone(), *new_window, cx)
            }
            WelcomeEvent::Quit => crate::workbench::quit(cx),
        }
    }

    /// Opens `path` chosen on the Welcome screen: in this window, or in a
    /// new process when `new_window`. Ignored once the window shows a
    /// workbench, as for the second `Open` of a double-click.
    fn open_from_welcome(
        this: &Rc<RefCell<Router>>,
        path: PathBuf,
        new_window: bool,
        cx: &mut App,
    ) {
        if !matches!(this.borrow().shown, Some(Shown::Welcome(_))) {
            return;
        }
        if new_window {
            if let Err(err) = process::spawn(&SpawnRequest::Path(path.clone()), cx) {
                tracing::error!(%err, path = %path.display(), "could not start luc");
            }
            return;
        }
        Router::open_path(this, &path, cx);
    }

    fn open_urls(this: &Rc<RefCell<Router>>, urls: &[String], cx: &mut App) {
        for path in open_requests::file_paths(urls) {
            Router::open_path(this, &path, cx);
        }
    }

    /// Opens `path` where [`open_requests::route`] says; while the Welcome
    /// screen shows, that is in its window.
    fn open_path(this: &Rc<RefCell<Router>>, path: &Path, cx: &mut App) {
        let path = match std::fs::canonicalize(path) {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "cannot open");
                return;
            }
        };
        let (route, handle) = match &this.borrow().shown {
            Some(Shown::Workbench(open, handle)) => {
                (open_requests::route(&path, Some(open)), Some(*handle))
            }
            _ => (open_requests::route(&path, None), None),
        };
        tracing::info!(path = %path.display(), ?route, "open request");
        match (route, handle) {
            (Route::ThisWindow, Some(handle)) => {
                let file = path.is_file().then(|| Arc::<Path>::from(path));
                handle
                    .update(cx, |workbench, window, cx| {
                        if let Some(file) = file {
                            workbench.open_path(file, OpenMode::Pinned, cx);
                        }
                        window.activate_window();
                    })
                    .ok();
                cx.activate(true);
            }
            (Route::NewProcess, _) => open_requests::spawn_process(&path, cx),
            _ => {
                let resolved = (this.borrow().resolve)(path);
                match resolved {
                    Ok(launch) => Router::open_first(this, launch, cx),
                    Err(err) => tracing::error!(err, "could not open"),
                }
            }
        }
    }
}

/// Adds `entry` to the recent list in the background, when the app has a
/// data directory, then rebuilds the menu bar's Open Recent.
fn record_recent(entry: RecentEntry, cx: &mut App) {
    let Some(data_dir) = app_data_dir(cx) else {
        return;
    };
    cx.spawn(async move |cx| {
        let recorded = cx
            .background_spawn(async move { recent::record(&data_dir, entry) })
            .await;
        if let Err(err) = recorded {
            tracing::warn!(%err, "could not update the recent list");
        }
        cx.update(app_menu::refresh);
    })
    .detach();
}

/// The app data directory ([`RecentStore`]): the recent list and the
/// workspace state live there.
fn app_data_dir(cx: &App) -> Option<PathBuf> {
    cx.try_global::<RecentStore>()
        .and_then(|store| store.data_dir.clone())
}

/// Asks for a folder to open; quits on Cancel unless a window opened
/// meanwhile.
fn pick_folder(router: &Rc<RefCell<Router>>, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open".into()),
    });
    let router = router.clone();
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
            Some(folder) => Router::open_path(&router, &folder, cx),
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
    cx.set_global(Spawner::default());
    app_menu::install(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Options for the app's window: zoomed to fill the screen (what
/// double-clicking the title bar does on macOS). The windowed size is what
/// un-zooming returns to.
fn maximized_window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
    let mut options = window_options(bounds);
    // GPUI's test platform cannot zoom a window.
    if !cx.background_executor().is_test() {
        options.window_bounds = Some(WindowBounds::Maximized(bounds));
    }
    options
}

/// Opens the workbench window on `launch`; quits if it cannot.
fn open_workbench(launch: Launch, cx: &mut App) -> Option<WindowHandle<Workbench>> {
    let opened = cx.open_window(maximized_window_options(cx), |window, cx| {
        build_workbench(launch, window, cx)
    });
    match opened {
        Ok(handle) => {
            cx.activate(true);
            Some(handle)
        }
        Err(err) => {
            tracing::error!(%err, "could not open the window");
            cx.quit();
            None
        }
    }
}

/// Makes the Welcome window show the workbench on `launch`, keeping its
/// place and size. `None` when the window closed meanwhile.
fn replace_welcome(
    welcome: WindowHandle<WelcomeView>,
    launch: Launch,
    cx: &mut App,
) -> Option<WindowHandle<Workbench>> {
    let any = *welcome;
    match any.update(cx, |_, window, cx| {
        build_workbench(launch, window, cx);
        window.activate_window();
        cx.activate(true);
    }) {
        // The window's handle still names its first root type.
        Ok(()) => Some(WindowHandle::new(any.window_id())),
        Err(err) => {
            tracing::error!(%err, "could not show the workbench");
            None
        }
    }
}

/// Makes the workbench on `launch` the root of `window`, opens its file,
/// and runs the startup work once its first frame is presented.
fn build_workbench(launch: Launch, window: &mut Window, cx: &mut App) -> Entity<Workbench> {
    let Launch {
        root,
        file,
        single_file,
        on_first_frame,
    } = launch;
    let data_dir = app_data_dir(cx);
    let workbench = window.replace_root(cx, |window, cx| match (&file, data_dir) {
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
}

/// The router the test hooks below drive, made on first use. It resolves
/// a folder to itself and a file to a single-file window, and reads the
/// user configuration from `config` in the app data directory, not the
/// developer's own.
struct TestRouter(Rc<RefCell<Router>>);

impl Global for TestRouter {}

fn test_router(cx: &mut App) -> Rc<RefCell<Router>> {
    if let Some(router) = cx.try_global::<TestRouter>() {
        return router.0.clone();
    }
    let resolve: Resolve = Box::new(|path: PathBuf| {
        Ok(if path.is_dir() {
            Launch {
                root: path,
                file: None,
                single_file: false,
                on_first_frame: None,
            }
        } else {
            Launch {
                root: path.parent().unwrap_or(Path::new("/")).to_path_buf(),
                file: Some(path),
                single_file: true,
                on_first_frame: None,
            }
        })
    });
    let user_dir = app_data_dir(cx).unwrap_or_default().join("config");
    let router = Rc::new(RefCell::new(Router {
        shown: None,
        resolve,
        user_dir,
    }));
    cx.set_global(TestRouter(router.clone()));
    router
}

/// Opens the Welcome screen through the app's router; for tests.
#[doc(hidden)]
pub fn open_welcome_for_test(cx: &mut App) -> WindowHandle<WelcomeView> {
    let router = test_router(cx);
    Router::open_welcome(&router, cx);
    match router.borrow().shown {
        Some(Shown::Welcome(handle)) => handle,
        _ => panic!("the Welcome window opened"),
    }
}

/// Opens `launch` through the app's router, as `luc <path>` does; for
/// tests.
#[doc(hidden)]
pub fn open_launch_for_test(launch: Launch, cx: &mut App) {
    let router = test_router(cx);
    Router::open_first(&router, launch, cx);
}

/// The folder of the workbench the router shows; `None` while it shows
/// Welcome or nothing. For tests.
#[doc(hidden)]
pub fn router_shown_root_for_test(cx: &mut App) -> Option<PathBuf> {
    let router = test_router(cx);
    match &router.borrow().shown {
        Some(Shown::Workbench(open, _)) => Some(open.root().to_path_buf()),
        _ => None,
    }
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
