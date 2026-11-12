//! Starts the app for `luc`: the configuration, themes and keymap, then
//! one workbench window on a folder, asking for the folder when the app
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
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, PathPromptOptions, WindowBounds,
    WindowHandle, actions, px, size,
};
use gpui_platform::application;
use lucerna_extension::store::Store;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::{Associations, LanguageRegistry};
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};

use crate::extensions::{
    ExtensionService, WasmGrammarValidator, extension_languages, extensions_root,
};
use crate::open_requests::{self, OpenWindow, Route};
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
/// otherwise asks for a folder; Cancel quits. Paths the platform asks to
/// open later go through [`open_requests::route`], resolved by `resolve`.
pub fn run(launch: Option<Launch>, resolve: Resolve) {
    let (requests_tx, mut requests) = mpsc::unbounded::<Vec<String>>();
    let app = application();
    app.on_open_urls(move |urls| {
        requests_tx.unbounded_send(urls).ok();
    });
    app.run(move |cx: &mut App| {
        start(cx);
        let router = Rc::new(RefCell::new(Router {
            window: None,
            resolve,
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
                    Either::Right(_) => cx.update(|cx| pick_folder(&router, cx)),
                }
            }
            while let Some(urls) = requests.next().await {
                cx.update(|cx| Router::open_urls(&router, &urls, cx));
            }
        })
        .detach();
    });
}

/// The process's one window, and how to open paths the platform sends.
struct Router {
    window: Option<(OpenWindow, WindowHandle<Workbench>)>,
    resolve: Resolve,
}

impl Router {
    fn open_first(this: &Rc<RefCell<Router>>, launch: Launch, cx: &mut App) {
        let shown = match (&launch.file, launch.single_file) {
            (Some(file), true) => OpenWindow::single_file(file.clone()),
            _ => OpenWindow::folder(launch.root.clone()),
        };
        if let Some(handle) = open_workbench(launch, cx) {
            this.borrow_mut().window = Some((shown, handle));
        }
    }

    fn open_urls(this: &Rc<RefCell<Router>>, urls: &[String], cx: &mut App) {
        for path in open_requests::file_paths(urls) {
            Router::open_path(this, &path, cx);
        }
    }

    fn open_path(this: &Rc<RefCell<Router>>, path: &Path, cx: &mut App) {
        let path = match std::fs::canonicalize(path) {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "cannot open");
                return;
            }
        };
        let (route, handle) = {
            let router = this.borrow();
            let shown = router.window.as_ref().map(|(shown, _)| shown);
            let handle = router.window.as_ref().map(|(_, handle)| *handle);
            (open_requests::route(&path, shown), handle)
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
            (Route::NewProcess, _) => open_requests::spawn_process(&path),
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
    install_app_menu(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Opens the workbench window on `launch`; quits if it cannot.
fn open_workbench(launch: Launch, cx: &mut App) -> Option<WindowHandle<Workbench>> {
    let Launch {
        root,
        file,
        single_file,
        on_first_frame,
    } = launch;
    watch_config(ConfigPaths::for_root(&root), cx);
    // Open zoomed to fill the screen (what double-clicking the title bar does
    // on macOS). The windowed size is what un-zooming returns to.
    let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
    let mut options = window_options(bounds);
    options.window_bounds = Some(WindowBounds::Maximized(bounds));
    let opened = cx.open_window(options, |window, cx| {
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

actions!(lucerna, [Quit, Hide, Minimize]);

/// The macOS app menu: Lucerna → Hide (⌘H), Quit (⌘Q), and Window →
/// Minimize (⌘M). The menu shows the shortcuts GPUI's keymap binds to its
/// actions; ⌘Q also resolves through the user keymap to
/// `workbench.action.quit`. Both quit the same way: each window first asks
/// about unsaved changes, then the layout is saved by the workbench's
/// `on_app_quit` hook.
fn install_app_menu(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| crate::workbench::quit(cx));
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
