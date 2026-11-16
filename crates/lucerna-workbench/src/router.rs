//! The app's windows, all in one process (spec §6.2–6.3 of the
//! multi-window design): what each shows, which was used last, and
//! [`open`], the one way a path opens, whoever asks (`luc`, Finder, the
//! Welcome screen, Open Recent, the Open dialog, New Window).
//!
//! Every change to the list of windows goes through [`Router::set`],
//! [`Router::remove`] or [`Router::touch`] and then [`changed`], which
//! also writes the list to `windows.json` ([`save_session`]). A start
//! reopens that list with [`restore`]; paths `luc` or the platform ask
//! for meanwhile wait until it is done (spec M14).

use std::cell::RefCell;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, Subscription, Window,
    WindowBounds, WindowHandle, WindowId, WindowOptions, px, size,
};

use crate::app_menu;
use crate::extensions::ExtensionService;
use crate::recent::{self, RecentEntry, RecentKind, RecentStore};
use crate::session::{self, Session, SessionWindow, session_path};
pub use crate::window_choice::WindowKind;
use crate::window_choice::{self, Choice, WindowInfo};
use crate::workspace_state::tilde_path;
use crate::{
    ConfigPaths, ConfigState, Host, OnboardingEvent, OnboardingPage, OpenMode, WelcomeEvent,
    WelcomeView, WindowSettings, Workbench, status_message, watch_config, window_options,
};

/// What a workbench window opens on.
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

impl Launch {
    /// What `luc <path>` opens: a folder as itself, a file alone (rooted
    /// at its folder), both canonical.
    ///
    /// # Errors
    /// `path not found: …` when nothing is at `path`.
    pub fn for_path(path: PathBuf) -> Result<Launch, String> {
        let canonical = std::fs::canonicalize(&path).map_err(|err| match err.kind() {
            ErrorKind::NotFound => format!("path not found: {}", path.display()),
            _ => format!("{}: {err}", path.display()),
        })?;
        Ok(if canonical.is_dir() {
            Launch {
                root: canonical,
                file: None,
                single_file: false,
                on_first_frame: None,
            }
        } else {
            Launch {
                root: canonical.parent().unwrap_or(Path::new("/")).to_path_buf(),
                file: Some(canonical),
                single_file: true,
                on_first_frame: None,
            }
        })
    }

    /// The path the window is on: the file alone, else the folder.
    fn path(&self) -> &Path {
        match (&self.file, self.single_file) {
            (Some(file), true) => file,
            _ => &self.root,
        }
    }

    fn kind(&self) -> WindowKind {
        match (&self.file, self.single_file) {
            (Some(file), true) => WindowKind::File(file.clone()),
            _ => WindowKind::Folder(self.root.clone()),
        }
    }
}

/// Where a request wants its paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Wherever the rules of [`window_choice::choose`] put them.
    Auto,
    /// A new window, unless the path is open in one already.
    NewWindow,
    /// In place of what this window shows (spec W8, W10), unless the path
    /// is open in a window already. Its workbench has already asked to
    /// save.
    ReplaceIn(AnyWindowHandle),
}

/// Who asks to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `luc <path>` in a terminal.
    Cli,
    /// The platform: Finder's "Open With", the Dock.
    Finder,
    /// The app's own UI: Welcome, Open Recent, the Open dialog.
    Menu,
    /// Reopening the last session's windows.
    Restore,
}

/// Paths to open, from [`Source`], where [`Target`] says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub paths: Vec<PathBuf>,
    pub target: Target,
    pub source: Source,
}

/// Called with the result of an [`open_then`], once the paths opened
/// (after the restore when the request waited for it).
pub type OpenDone = Box<dyn FnOnce(Result<(), String>, &mut App)>;

/// What a router window shows; [`WindowKind`] without telling Welcome from
/// the Get Started page.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    Welcome,
    Onboarding,
    Workbench(WindowKind),
}

impl Shown {
    fn kind(&self) -> WindowKind {
        match self {
            Shown::Welcome | Shown::Onboarding => WindowKind::Welcome,
            Shown::Workbench(kind) => kind.clone(),
        }
    }
}

/// Follows a window's activation, for [`Entry::last_active`]. The entity
/// outlives the window's roots.
struct ActivationProbe;

/// One window of the router.
struct Entry {
    handle: AnyWindowHandle,
    shown: Shown,
    /// Larger is used more recently.
    last_active: u64,
    _activation: Option<(Entity<ActivationProbe>, Subscription)>,
}

/// The app's windows.
#[derive(Default)]
struct Router {
    entries: Vec<Entry>,
    /// The last `last_active` given out.
    clock: u64,
    /// The extension registry check is scheduled once per app, after the
    /// first workbench's first frame.
    startup_refresh_scheduled: bool,
    /// The last session's windows are being reopened: requests from `luc`
    /// and the platform wait in `queue`, and the session is not written.
    restoring: bool,
    queue: Vec<(OpenRequest, Option<OpenDone>)>,
    /// The app is quitting: its windows close, but the session keeps them.
    quitting: bool,
    /// What the last background write wrote, so the same list is not
    /// written twice.
    last_saved: Option<Session>,
}

impl Router {
    fn index(&self, handle: AnyWindowHandle) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.handle.window_id() == handle.window_id())
    }

    fn infos(&self) -> Vec<WindowInfo> {
        self.entries
            .iter()
            .map(|e| WindowInfo {
                kind: e.shown.kind(),
                last_active: e.last_active,
            })
            .collect()
    }

    fn shown(&self, handle: AnyWindowHandle) -> Option<Shown> {
        self.index(handle).map(|i| self.entries[i].shown.clone())
    }

    /// Records that `handle` shows `shown`, adding the window when new.
    /// Whether it was new.
    fn set(&mut self, handle: AnyWindowHandle, shown: Shown) -> bool {
        match self.index(handle) {
            Some(i) => {
                self.entries[i].shown = shown;
                false
            }
            None => {
                self.entries.push(Entry {
                    handle,
                    shown,
                    last_active: 0,
                    _activation: None,
                });
                true
            }
        }
    }

    fn remove(&mut self, id: WindowId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.handle.window_id() != id);
        self.entries.len() != before
    }

    /// Makes `handle` the most recently used window.
    fn touch(&mut self, handle: AnyWindowHandle) -> bool {
        let Some(i) = self.index(handle) else {
            return false;
        };
        if self.entries[i].last_active == self.clock && self.clock > 0 {
            return false;
        }
        self.clock += 1;
        self.entries[i].last_active = self.clock;
        true
    }
}

#[derive(Default)]
struct RouterGlobal(Rc<RefCell<Router>>);

impl Global for RouterGlobal {}

/// The app's router, made on first use; then it follows closed windows.
fn router(cx: &mut App) -> Rc<RefCell<Router>> {
    if !cx.has_global::<RouterGlobal>() {
        cx.default_global::<RouterGlobal>();
        cx.on_app_quit(|cx| {
            router(cx).borrow_mut().quitting = true;
            async {}
        })
        .detach();
        cx.on_window_closed(|cx, id| {
            status_message::forget(id, cx);
            let removed = router(cx).borrow_mut().remove(id);
            if removed {
                changed(cx);
            }
        })
        .detach();
    }
    cx.global::<RouterGlobal>().0.clone()
}

/// After every change to the router's windows: the menu bar follows the
/// active window's kind, and `windows.json` the windows.
fn changed(cx: &mut App) {
    app_menu::refresh(cx);
    save_session(cx);
}

/// Writes the workbench windows, most recently used first, to
/// `windows.json` in the background (spec §6.6), so that even a crash
/// leaves the list right. Nothing while a restore runs (it writes once
/// done) or the app quits (`session::save_now` wrote the list), nor when
/// the list did not change since the last write.
pub fn save_session(cx: &mut App) {
    let Some(data_dir) = app_data_dir(cx) else {
        return;
    };
    let router = router(cx);
    let session = session::current(cx);
    {
        let mut router = router.borrow_mut();
        if router.restoring || router.quitting || router.last_saved.as_ref() == Some(&session) {
            return;
        }
        router.last_saved = Some(session.clone());
    }
    let write = session::next_write();
    cx.background_spawn(async move {
        if let Err(err) = session::save_in_order(&session_path(&data_dir), &session, write) {
            tracing::warn!(%err, "could not write the window list");
        }
    })
    .detach();
}

/// From now until [`finish_restore`], requests from `luc` and the
/// platform wait (spec M14).
pub fn begin_restore(cx: &mut App) {
    router(cx).borrow_mut().restoring = true;
}

/// Reopens the last session's `windows` (most recently used first): the
/// oldest first, so that the most recently used ends up in front and
/// used last, as Zed does. A file comes back alone, not as a tab of a
/// folder window. Then [`finish_restore`].
pub fn restore(windows: Vec<SessionWindow>, cx: &mut App) {
    begin_restore(cx);
    for window in windows.into_iter().rev() {
        match Launch::for_path(window.path.clone()) {
            Ok(launch) => {
                open_launch(launch, Target::NewWindow, cx);
            }
            Err(err) => {
                tracing::warn!(err, path = %window.path.display(), "could not reopen the window");
            }
        }
    }
    finish_restore(cx);
}

/// Ends a restore: writes the session, then opens the requests that
/// waited, in order. A path the restore already opened is only focused
/// (rule 1 of spec §6.3).
pub fn finish_restore(cx: &mut App) {
    let queue = {
        let router = router(cx);
        let mut router = router.borrow_mut();
        router.restoring = false;
        std::mem::take(&mut router.queue)
    };
    changed(cx);
    for (request, done) in queue {
        let result = open_now(request, cx);
        if let Some(done) = done {
            done(result, cx);
        }
    }
}

/// The router's windows and what they show, most recently used first.
pub fn windows(cx: &App) -> Vec<(AnyWindowHandle, WindowKind)> {
    let Some(router) = cx.try_global::<RouterGlobal>() else {
        return Vec::new();
    };
    let router = router.0.borrow();
    let mut entries: Vec<&Entry> = router.entries.iter().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.last_active));
    entries
        .into_iter()
        .map(|e| (e.handle, e.shown.kind()))
        .collect()
}

/// Opens `request`'s paths, each where [`window_choice::choose`] says,
/// then records them as recently opened. A path that cannot be opened is
/// skipped and logged; the UI says why in the status bar. While a
/// [`restore`] runs, a request from `luc` or the platform waits for it
/// and this returns `Ok`; [`open_then`] reports the real result.
///
/// # Errors
/// What went wrong, one line per path, for `luc` to print.
pub fn open(request: OpenRequest, cx: &mut App) -> Result<(), String> {
    match wait_for_restore(request, None, cx) {
        Some((request, _)) => open_now(request, cx),
        None => Ok(()),
    }
}

/// [`open`], then `done` with its result: at once, or once the restore
/// the request waits for is over.
pub fn open_then(request: OpenRequest, done: OpenDone, cx: &mut App) {
    if let Some((request, done)) = wait_for_restore(request, Some(done), cx) {
        let result = open_now(request, cx);
        if let Some(done) = done {
            done(result, cx);
        }
    }
}

/// Answers a request `luc` sent over the socket ([`crate::cli_socket`]):
/// opens its paths ([`Source::Cli`], [`Target::Auto`]) and replies with
/// the result, after the restore when one runs. A request from a newer
/// `luc` is refused.
#[cfg(unix)]
pub fn answer_cli(
    request: crate::cli_socket::CliRequest,
    reply: futures::channel::oneshot::Sender<crate::cli_socket::CliReply>,
    cx: &mut App,
) {
    use crate::cli_socket::{CliReply, reply_for_version};

    if let Some(refused) = reply_for_version(request.version) {
        tracing::warn!(version = request.version, "a newer luc asked to open");
        reply.send(refused).ok();
        return;
    }
    let request = OpenRequest {
        paths: request.paths_to_open(),
        target: Target::Auto,
        source: Source::Cli,
    };
    let done: OpenDone = Box::new(move |result, _| {
        // `luc` may have given up waiting.
        reply.send(CliReply::from(result)).ok();
    });
    open_then(request, done, cx);
}

/// Queues a request from `luc` or the platform while a restore runs
/// (`None`); otherwise gives it back to open now.
fn wait_for_restore(
    request: OpenRequest,
    done: Option<OpenDone>,
    cx: &mut App,
) -> Option<(OpenRequest, Option<OpenDone>)> {
    if matches!(request.source, Source::Cli | Source::Finder) {
        let router = router(cx);
        let mut router = router.borrow_mut();
        if router.restoring {
            tracing::info!(paths = ?request.paths, "waiting for the restore");
            router.queue.push((request, done));
            return None;
        }
    }
    Some((request, done))
}

/// [`open`] without waiting for a restore.
fn open_now(request: OpenRequest, cx: &mut App) -> Result<(), String> {
    let mut errors = Vec::new();
    for path in request.paths {
        match Launch::for_path(path.clone()) {
            Ok(launch) => {
                if open_launch(launch, request.target, cx).is_none() {
                    let err = format!("could not open a window for {}", path.display());
                    tracing::error!(err, source = ?request.source, "could not open");
                    errors.push(err);
                }
            }
            Err(err) => {
                tracing::error!(err, path = %path.display(), source = ?request.source, "could not open");
                if request.source == Source::Menu {
                    let what = tilde_path(&path, std::env::home_dir().as_deref());
                    // The window the user is in, the one most recently used.
                    let window = windows(cx).first().map(|(handle, _)| handle.window_id());
                    status_message::show_or_log(
                        window,
                        format!("Could not open {what}: {err}"),
                        cx,
                    );
                }
                errors.push(err);
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Opens `launch` where the rules say; the window it ends up in.
pub(crate) fn open_launch(launch: Launch, target: Target, cx: &mut App) -> Option<AnyWindowHandle> {
    let router = router(cx);
    let target = match target {
        Target::Auto => window_choice::Target::Auto,
        Target::NewWindow => window_choice::Target::NewWindow,
        Target::ReplaceIn(handle) => match index_or_adopt(handle, cx) {
            Some(index) => window_choice::Target::ReplaceIn(index),
            None => window_choice::Target::NewWindow,
        },
    };
    let (windows, handles): (Vec<WindowInfo>, Vec<AnyWindowHandle>) = {
        let router = router.borrow();
        (
            router.infos(),
            router.entries.iter().map(|e| e.handle).collect(),
        )
    };
    let path = launch.path().to_path_buf();
    let choice = window_choice::choose(&windows, &path, !launch.single_file, target);
    tracing::info!(path = %path.display(), ?choice, "open");
    let handle = match choice {
        Choice::Focus(index) => {
            activate(handles[index], cx);
            Some(handles[index])
        }
        Choice::OpenTab(index) => {
            let handle = handles[index];
            let file: Arc<Path> = Arc::from(path.as_path());
            WindowHandle::<Workbench>::new(handle.window_id())
                .update(cx, |workbench, window, cx| {
                    workbench.open_path(file, OpenMode::Pinned, cx);
                    window.activate_window();
                })
                .ok();
            cx.activate(true);
            Some(handle)
        }
        Choice::ReplaceRoot(index) => {
            let handle = handles[index];
            let entry = recent_entry(&launch);
            let kind = launch.kind();
            replace_with_workbench(handle, launch, cx).then(|| {
                router.borrow_mut().set(handle, Shown::Workbench(kind));
                record_recent(entry, cx);
                handle
            })
        }
        Choice::NewWindow => {
            let entry = recent_entry(&launch);
            let kind = launch.kind();
            let handle = open_window(cx, |window, cx| build_workbench(launch, window, cx))?;
            register(handle, Shown::Workbench(kind), cx);
            record_recent(entry, cx);
            Some(handle)
        }
    };
    if let Some(handle) = handle {
        router.borrow_mut().touch(handle);
    }
    changed(cx);
    handle
}

/// The window an Open of `path` would only bring to the front: a folder
/// window on it, or a single-file window on it (rule 1 of spec §6.3).
pub(crate) fn window_showing(path: &Path, cx: &mut App) -> Option<AnyWindowHandle> {
    let path = std::fs::canonicalize(path).ok()?;
    let kind = if path.is_dir() {
        WindowKind::Folder(path)
    } else {
        WindowKind::File(path)
    };
    let router = router(cx);
    let router = router.borrow();
    router
        .entries
        .iter()
        .filter(|e| e.shown.kind() == kind)
        .max_by_key(|e| e.last_active)
        .map(|e| e.handle)
}

/// Brings `handle` to the front and makes it the most recently used.
pub(crate) fn activate(handle: AnyWindowHandle, cx: &mut App) {
    handle
        .update(cx, |_, window, _| window.activate_window())
        .ok();
    cx.activate(true);
    let touched = router(cx).borrow_mut().touch(handle);
    if touched {
        changed(cx);
    }
}

/// Opens the Welcome screen: in place of what `ReplaceIn`'s window shows
/// (spec W10, O5), else in a new window.
pub fn open_welcome(target: Target, cx: &mut App) {
    let handle = match target {
        Target::ReplaceIn(handle) => {
            match handle.update(cx, |_, window, cx| {
                welcome_root(window, cx);
            }) {
                Ok(()) => handle,
                Err(err) => {
                    tracing::error!(%err, "could not show the Welcome screen");
                    return;
                }
            }
        }
        Target::Auto | Target::NewWindow => match open_window(cx, welcome_root) {
            Some(handle) => handle,
            None => return,
        },
    };
    register(handle, Shown::Welcome, cx);
    router(cx).borrow_mut().touch(handle);
    changed(cx);
}

/// Opens the Get Started page in a new window.
pub fn open_onboarding(cx: &mut App) {
    let Some(handle) = open_window(cx, onboarding_root) else {
        return;
    };
    register(handle, Shown::Onboarding, cx);
    router(cx).borrow_mut().touch(handle);
    changed(cx);
}

/// Shows the Get Started page in place of the Welcome screen in `handle`
/// (spec §6 of the Welcome design); nothing once it shows anything else.
fn replace_with_onboarding(handle: AnyWindowHandle, cx: &mut App) {
    if router(cx).borrow().shown(handle) != Some(Shown::Welcome) {
        return;
    }
    let replaced = handle.update(cx, |_, window, cx| {
        onboarding_root(window, cx);
    });
    match replaced {
        Ok(()) => {
            register(handle, Shown::Onboarding, cx);
            changed(cx);
        }
        Err(err) => tracing::error!(%err, "could not show the Get Started page"),
    }
}

/// Makes the Get Started page the root of `window`; this router handles
/// its events.
fn onboarding_root(window: &mut Window, cx: &mut App) -> Entity<OnboardingPage> {
    let handle = window.window_handle();
    let page = window.replace_root(cx, |window, cx| {
        // A window without a folder: the app's settings alone.
        let settings = cx.new(|cx| WindowSettings::new(None, cx));
        OnboardingPage::new(Host::Window, settings, window, cx)
    });
    cx.subscribe(&page, move |_, event: &OnboardingEvent, cx| {
        on_onboarding_event(handle, event, cx)
    })
    .detach();
    page
}

/// Makes the Welcome screen the root of `window`; this router handles its
/// events.
fn welcome_root(window: &mut Window, cx: &mut App) -> Entity<WelcomeView> {
    let handle = window.window_handle();
    let view = window.replace_root(cx, WelcomeView::new);
    cx.subscribe(&view, move |_, event: &WelcomeEvent, cx| {
        on_welcome_event(handle, event, cx)
    })
    .detach();
    view
}

fn on_welcome_event(handle: AnyWindowHandle, event: &WelcomeEvent, cx: &mut App) {
    // As for the second `Open` of a double-click, once the window shows
    // something else.
    if router(cx).borrow().shown(handle) != Some(Shown::Welcome) {
        return;
    }
    match event {
        WelcomeEvent::Open { path, new_window } => {
            let target = if *new_window {
                Target::NewWindow
            } else {
                Target::ReplaceIn(handle)
            };
            let request = OpenRequest {
                paths: vec![path.clone()],
                target,
                source: Source::Menu,
            };
            open(request, cx).ok();
        }
        WelcomeEvent::OpenOnboarding => replace_with_onboarding(handle, cx),
        WelcomeEvent::Quit => crate::workbench::quit(cx),
    }
}

fn on_onboarding_event(handle: AnyWindowHandle, event: &OnboardingEvent, cx: &mut App) {
    match event {
        OnboardingEvent::Finish => {
            if router(cx).borrow().shown(handle) == Some(Shown::Onboarding) {
                open_welcome(Target::ReplaceIn(handle), cx);
            }
        }
        OnboardingEvent::OpenSettingsInNewWindow(path) => {
            let request = OpenRequest {
                paths: vec![path.clone()],
                target: Target::NewWindow,
                source: Source::Menu,
            };
            open(request, cx).ok();
        }
        // Only a tab asks this, of its workbench.
        OnboardingEvent::OpenSettingsInWorkbench => {}
    }
}

/// Records what `handle` shows; a new window also gets an activation
/// observer that keeps its `last_active`.
fn register(handle: AnyWindowHandle, shown: Shown, cx: &mut App) {
    let routed = router(cx);
    let added = routed.borrow_mut().set(handle, shown);
    if !added {
        return;
    }
    let probe = cx.new(|_| ActivationProbe);
    let subscription = handle.update(cx, |_, window, cx| {
        probe.update(cx, |_, cx| {
            cx.observe_window_activation(window, move |_, window, cx| {
                if window.is_window_active() {
                    let touched = router(cx).borrow_mut().touch(handle);
                    if touched {
                        changed(cx);
                    }
                }
            })
        })
    });
    match subscription {
        Ok(subscription) => {
            let mut routed = routed.borrow_mut();
            if let Some(i) = routed.index(handle) {
                routed.entries[i]._activation = Some((probe, subscription));
            }
        }
        Err(err) => tracing::warn!(%err, "cannot follow the window's activation"),
    }
}

/// The index of `handle` in the router; a window the router did not open
/// (tests build workbenches directly) is adopted from its root.
fn index_or_adopt(handle: AnyWindowHandle, cx: &mut App) -> Option<usize> {
    let router = router(cx);
    if let Some(index) = router.borrow().index(handle) {
        return Some(index);
    }
    let id = handle.window_id();
    let shown = if let Ok(kind) =
        WindowHandle::<Workbench>::new(id).read_with(cx, |wb, _| match wb.opened_paths().get(1) {
            Some(file) => WindowKind::File(file.clone()),
            None => WindowKind::Folder(wb.root().to_path_buf()),
        }) {
        Shown::Workbench(kind)
    } else if WindowHandle::<OnboardingPage>::new(id).read(cx).is_ok() {
        Shown::Onboarding
    } else if WindowHandle::<WelcomeView>::new(id).read(cx).is_ok() {
        Shown::Welcome
    } else {
        return None;
    };
    register(handle, shown, cx);
    router.borrow().index(handle)
}

/// Opens a window like the workbench's on the root `build` makes; `None`
/// (logged) when the platform cannot. The start quits when its first
/// window fails (`app::start_failed`); later failures leave the app as is.
fn open_window<V: 'static + gpui::Render>(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<AnyWindowHandle> {
    match cx.open_window(maximized_window_options(cx), build) {
        Ok(handle) => {
            cx.activate(true);
            Some(handle.into())
        }
        Err(err) => {
            tracing::error!(%err, "could not open the window");
            None
        }
    }
}

/// Options for the app's windows: zoomed to fill the screen (what
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

/// Makes `handle` (a Welcome window, or a workbench being replaced) show
/// the workbench on `launch`, keeping its place and size, without a
/// window animation. The root it showed is released. `false` when the
/// window closed meanwhile.
fn replace_with_workbench(handle: AnyWindowHandle, launch: Launch, cx: &mut App) -> bool {
    match handle.update(cx, |_, window, cx| {
        build_workbench(launch, window, cx);
        window.activate_window();
        cx.activate(true);
    }) {
        Ok(()) => true,
        Err(err) => {
            tracing::error!(%err, "could not show the workbench");
            false
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
    let refresh = {
        let router = router(cx);
        let mut router = router.borrow_mut();
        !std::mem::replace(&mut router.startup_refresh_scheduled, true)
    };
    // Next-frame callbacks run before that frame draws, so the
    // second one runs once the first frame has been presented.
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |_, cx| {
            if refresh {
                ExtensionService::schedule_startup_refresh(cx);
            }
            if let Some(callback) = on_first_frame {
                callback();
            }
        });
    });
    workbench
}

fn recent_entry(launch: &Launch) -> RecentEntry {
    match (&launch.file, launch.single_file) {
        (Some(file), true) => RecentEntry {
            path: file.clone(),
            kind: RecentKind::File,
        },
        _ => RecentEntry {
            path: launch.root.clone(),
            kind: RecentKind::Folder,
        },
    }
}

/// Adds `entry` to the recent list in the background, when the app has a
/// data directory, then rebuilds the menu bar's Open Recent. The system's
/// recent documents get it too, as Zed and VS Code do: on macOS that is
/// what App Exposé (Ctrl+Down) and the Dock menu list; a no-op elsewhere.
fn record_recent(entry: RecentEntry, cx: &mut App) {
    cx.add_recent_document(&entry.path);
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
pub(crate) fn app_data_dir(cx: &App) -> Option<PathBuf> {
    cx.try_global::<RecentStore>()
        .and_then(|store| store.data_dir.clone())
}

/// Opens `paths` through the router, as `luc` would, and reads the user
/// configuration from `config` in the app data directory, not the
/// developer's own; for tests.
#[doc(hidden)]
pub fn open_for_test(paths: Vec<PathBuf>, target: Target, cx: &mut App) {
    if !cx.has_global::<ConfigState>() {
        let user_dir = app_data_dir(cx).unwrap_or_default().join("config");
        // As `app::run` does, once.
        watch_config(ConfigPaths::user_only_with_user_dir(user_dir), cx);
    }
    let request = OpenRequest {
        paths,
        target,
        source: Source::Cli,
    };
    if let Err(err) = open(request, cx) {
        tracing::error!(err, "could not open");
    }
}
