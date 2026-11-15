//! Starts the app for `luc`: the configuration, themes and keymap, then
//! one workbench window on a folder, or the Welcome screen (the Get
//! Started page on the first run) when the app bundle is started from
//! Finder or the Dock, and opening the paths the platform sends
//! ([`open_requests`]).

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
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, PathPromptOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, px, size,
};
use gpui_platform::application;
use lucerna_extension::store::Store;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::{Associations, LanguageRegistry};
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};

use crate::app_menu;
use crate::app_state_file::{mark_onboarded, onboarded};
use crate::extensions::{
    ExtensionService, WasmGrammarValidator, extension_languages, extensions_root,
};
use crate::open_requests::{self, OpenWindow, Route};
use crate::process::{self, SpawnRequest, Spawner};
use crate::recent::{self, RecentEntry, RecentKind, RecentList, RecentStore};
use crate::startup::{StartupScreen, startup_screen};
use crate::workspace_state::tilde_path;
use crate::{
    AppState, ConfigPaths, Host, OnboardingEvent, OnboardingPage, OpenMode, WelcomeEvent,
    WelcomeView, Workbench, WorkbenchEvent, default_data_dir, last_workspace,
    restores_last_workspace, startup_choice, status_message, watch_config, window_options,
};

/// What [`run`] opens.
pub struct Launch {
    /// The folder to show, already canonical: the workspace state file and
    /// tab paths are keyed by it.
    pub root: PathBuf,
    /// A file to open as a pinned tab.
    pub file: Option<PathBuf>,
    /// Open `file` alone: `root` is not scanned or watched.
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
/// otherwise reopens the last workspace, shows the Get Started page on the
/// first run, or the Welcome screen, or asks for a folder when
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

/// With no folder to open: reads the user's configuration, then opens
/// what [`first_screen`] picks. The Get Started page is remembered as
/// shown as soon as it opens (spec O3).
fn open_without_folder(router: &Rc<RefCell<Router>>, cx: &mut App) {
    let user_dir = router.borrow().user_dir.clone();
    watch_config(ConfigPaths::user_only_with_user_dir(user_dir), cx);
    let data_dir = app_data_dir(cx);
    match first_screen(data_dir.as_deref(), &cx.global::<AppState>().settings) {
        StartupScreen::Restore(path) => {
            tracing::info!(path = %path.display(), "reopening the last workspace");
            Router::open_path(router, &path, cx);
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
            Router::open_onboarding(router, cx);
        }
        StartupScreen::Welcome => Router::open_welcome(router, cx),
        StartupScreen::FolderPrompt => pick_folder(router, cx),
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

/// What the process's one window shows.
enum Shown {
    Welcome(WindowHandle<WelcomeView>),
    Onboarding(WindowHandle<OnboardingPage>),
    Workbench(OpenWindow, WindowHandle<Workbench>),
}

impl Shown {
    fn window(&self) -> AnyWindowHandle {
        match self {
            Shown::Welcome(handle) => **handle,
            Shown::Onboarding(handle) => **handle,
            Shown::Workbench(_, handle) => **handle,
        }
    }
}

/// The process's one window, and how to open paths the platform sends.
struct Router {
    shown: Option<Shown>,
    resolve: Resolve,
    /// The user configuration directory ([`ConfigPaths::user_dir`]).
    user_dir: PathBuf,
}

impl Router {
    /// Opens the workbench on `launch`: in place of what the window shows
    /// (the Welcome screen, or a workbench being replaced, spec W8), else
    /// in a new window. Then records it as recently opened.
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
        let (shown, user_dir) = {
            let router = this.borrow();
            (
                router.shown.as_ref().map(Shown::window),
                router.user_dir.clone(),
            )
        };
        watch_config(ConfigPaths::with_user_dir(user_dir, &launch.root), cx);
        let handle = match shown {
            Some(window) => replace_root(this, window, launch, cx),
            None => open_workbench(this, launch, cx),
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
        let opened = cx.open_window(maximized_window_options(cx), |window, cx| {
            Router::welcome_view(this, window, cx)
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

    /// A Welcome view whose events this router handles.
    fn welcome_view(
        this: &Rc<RefCell<Router>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<WelcomeView> {
        let router = this.clone();
        let view = cx.new(|cx| WelcomeView::new(window, cx));
        cx.subscribe(&view, move |_, event: &WelcomeEvent, cx| {
            Router::on_welcome_event(&router, event, cx)
        })
        .detach();
        view
    }

    /// Opens the Get Started page in a window like the workbench's; quits
    /// if it cannot.
    fn open_onboarding(this: &Rc<RefCell<Router>>, cx: &mut App) {
        let opened = cx.open_window(maximized_window_options(cx), |window, cx| {
            let view = cx.new(|cx| OnboardingPage::new(Host::Window, window, cx));
            Router::route_onboarding(this, &view, cx);
            view
        });
        match opened {
            Ok(handle) => {
                cx.activate(true);
                this.borrow_mut().shown = Some(Shown::Onboarding(handle));
                // The menu bar enables the page's items.
                app_menu::refresh(cx);
            }
            Err(err) => {
                tracing::error!(%err, "could not open the window");
                cx.quit();
            }
        }
    }

    /// Makes this router handle the Get Started page's events.
    fn route_onboarding(this: &Rc<RefCell<Router>>, view: &Entity<OnboardingPage>, cx: &mut App) {
        let router = this.clone();
        cx.subscribe(view, move |_, event: &OnboardingEvent, cx| {
            Router::on_onboarding_event(&router, event, cx)
        })
        .detach();
    }

    fn on_onboarding_event(this: &Rc<RefCell<Router>>, event: &OnboardingEvent, cx: &mut App) {
        match event {
            OnboardingEvent::Finish => {
                if matches!(this.borrow().shown, Some(Shown::Onboarding(_))) {
                    Router::replace_with_welcome(this, cx);
                }
            }
            OnboardingEvent::OpenSettingsInNewWindow(path) => {
                if let Err(err) = process::spawn(&SpawnRequest::Path(path.clone()), cx) {
                    tracing::error!(%err, path = %path.display(), "could not start luc");
                }
            }
            // Only a tab asks this, of its workbench.
            OnboardingEvent::OpenSettingsInWorkbench => {}
        }
    }

    /// Shows the Get Started page in place of the Welcome screen (spec
    /// §6); nothing while the window shows anything else.
    fn replace_with_onboarding(this: &Rc<RefCell<Router>>, cx: &mut App) {
        let window = match &this.borrow().shown {
            Some(Shown::Welcome(handle)) => **handle,
            _ => return,
        };
        let replaced = window.update(cx, |_, window, cx| {
            let view = window.replace_root(cx, |window, cx| {
                OnboardingPage::new(Host::Window, window, cx)
            });
            Router::route_onboarding(this, &view, cx);
        });
        match replaced {
            Ok(()) => {
                this.borrow_mut().shown =
                    Some(Shown::Onboarding(WindowHandle::new(window.window_id())));
                app_menu::refresh(cx);
            }
            Err(err) => tracing::error!(%err, "could not show the Get Started page"),
        }
    }

    fn on_workbench_event(this: &Rc<RefCell<Router>>, event: &WorkbenchEvent, cx: &mut App) {
        match event {
            WorkbenchEvent::Replace(SpawnRequest::Path(path)) => {
                Router::replace_with_folder(this, path, cx)
            }
            WorkbenchEvent::Replace(SpawnRequest::Welcome) => {
                Router::replace_with_welcome(this, cx)
            }
        }
    }

    /// Shows the workbench on `path` in place of the one shown (spec W8).
    /// A path that cannot be opened keeps the window as it is and says why
    /// in the status bar.
    fn replace_with_folder(this: &Rc<RefCell<Router>>, path: &Path, cx: &mut App) {
        let resolved = std::fs::canonicalize(path)
            .map_err(|err| err.to_string())
            .and_then(|path| (this.borrow().resolve)(path));
        match resolved {
            Ok(launch) => Router::open_first(this, launch, cx),
            Err(err) => {
                tracing::error!(err, path = %path.display(), "could not open");
                let what = tilde_path(path, std::env::home_dir().as_deref());
                status_message::show(format!("Could not open {what}: {err}"), cx);
            }
        }
    }

    /// Shows the Welcome screen in place of the workbench (spec W10) or the
    /// Get Started page (spec O5), on the user configuration alone.
    fn replace_with_welcome(this: &Rc<RefCell<Router>>, cx: &mut App) {
        let (window, user_dir) = {
            let router = this.borrow();
            let Some(shown) = router.shown.as_ref() else {
                return;
            };
            (shown.window(), router.user_dir.clone())
        };
        watch_config(ConfigPaths::user_only_with_user_dir(user_dir), cx);
        let replaced = window.update(cx, |_, window, cx| {
            let view = window.replace_root(cx, WelcomeView::new);
            let router = this.clone();
            cx.subscribe(&view, move |_, event: &WelcomeEvent, cx| {
                Router::on_welcome_event(&router, event, cx)
            })
            .detach();
        });
        match replaced {
            Ok(()) => {
                this.borrow_mut().shown =
                    Some(Shown::Welcome(WindowHandle::new(window.window_id())));
                app_menu::refresh(cx);
            }
            Err(err) => tracing::error!(%err, "could not show the Welcome screen"),
        }
    }

    fn on_welcome_event(this: &Rc<RefCell<Router>>, event: &WelcomeEvent, cx: &mut App) {
        match event {
            WelcomeEvent::Open { path, new_window } => {
                Router::open_from_welcome(this, path.clone(), *new_window, cx)
            }
            WelcomeEvent::OpenOnboarding => Router::replace_with_onboarding(this, cx),
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
fn open_workbench(
    router: &Rc<RefCell<Router>>,
    launch: Launch,
    cx: &mut App,
) -> Option<WindowHandle<Workbench>> {
    let opened = cx.open_window(maximized_window_options(cx), |window, cx| {
        build_workbench(router, launch, window, cx)
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

/// Makes `any` (the Welcome window, or a workbench's being replaced) show
/// the workbench on `launch`, keeping its place and size, without a window
/// animation. The root it showed is released. `None` when the window
/// closed meanwhile.
fn replace_root(
    router: &Rc<RefCell<Router>>,
    any: AnyWindowHandle,
    launch: Launch,
    cx: &mut App,
) -> Option<WindowHandle<Workbench>> {
    match any.update(cx, |_, window, cx| {
        build_workbench(router, launch, window, cx);
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
/// and runs the startup work once its first frame is presented. `router`
/// handles its [`WorkbenchEvent`]s.
fn build_workbench(
    router: &Rc<RefCell<Router>>,
    launch: Launch,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Workbench> {
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
    let events = router.clone();
    cx.subscribe(&workbench, move |_, event: &WorkbenchEvent, cx| {
        Router::on_workbench_event(&events, event, cx)
    })
    .detach();
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
    let theme = themes.get_or_default("Lucerna Dark");
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
