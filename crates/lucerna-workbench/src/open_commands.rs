//! Opening folders and files from the workbench: Open Recent (`ctrl+r`),
//! Open…, Open File…, Open Folder…, Close Folder and Clear Recently
//! Opened, New Window. Another folder replaces this window's workbench
//! after the save prompt (spec W8; the app's [`router`] does the swap), or
//! opens in a new window of this process; a folder open in another window
//! already brings that window to the front. A file opens as a tab here, as
//! VS Code does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, PathPromptOptions, PromptLevel, Subscription,
    Task, Window, prelude::*,
};

use crate::app_menu;
use crate::recent::{self, RecentEntry, RecentKind, RecentStore};
use crate::router::{self, OpenRequest, Source, Target};
use crate::status_message;
use crate::workspace_state::tilde_path;
use crate::{CommandRegistry, OpenMode, RecentPicker, RecentPickerEvent, Workbench};

const CLEAR_MESSAGE: &str = "Do you want to clear all recently opened folders?";
const CLEAR_ANSWERS: [&str; 2] = ["Clear", "Cancel"];

/// What a workbench window shows in place of the workbench (spec W8,
/// W10), through [`Workbench::replace_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replacement {
    /// A workbench on this folder or file.
    Path(PathBuf),
    /// The Welcome screen.
    Welcome,
}

/// Opens a new window on the Welcome screen (spec M7).
pub(crate) const NEW_WINDOW: &str = "workbench.action.newWindow";

/// The shown Open Recent picker.
pub(crate) struct ShownRecentPicker {
    picker: Entity<RecentPicker>,
    _events: Subscription,
    focus_before: Option<FocusHandle>,
}

/// Runs "Open in Browser"; the palette lists it only for these files.
pub const OPEN_IN_BROWSER: &str = "workbench.action.openInBrowser";

const OPEN_IN_BROWSER_WHEN: &str = "resourceExtname == .html || resourceExtname == .htm";

/// The `file://` URL a browser opens for the HTML file at `path`; `None`
/// for a path that is not a file (relative).
pub fn browser_url(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// Whether `path` names an HTML file (extension `html` or `htm`, any case).
fn is_html(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(NEW_WINDOW, "New Window", |_, _, _, cx| {
        // Not inside this window's update: the router opens a window.
        cx.defer(|cx| router::open_welcome(Target::NewWindow, cx));
    });
    commands.register_when(
        OPEN_IN_BROWSER,
        "Open in Browser",
        OPEN_IN_BROWSER_WHEN,
        |wb, _, _, cx| {
            let path = wb
                .editor_groups()
                .read(cx)
                .model()
                .active_tab()
                .and_then(|tab| tab.path().cloned());
            if let Some(url) = path.filter(|p| is_html(p)).and_then(|p| browser_url(&p)) {
                cx.open_url(&url);
            }
        },
    );
    commands.register(
        "workbench.action.openRecent",
        "File: Open Recent…",
        |wb, _, window, cx| wb.show_recent_picker(window, cx),
    );
    commands.register(
        "workbench.action.files.openFileFolder",
        "File: Open…",
        |wb, _, window, cx| wb.prompt_and_open(true, true, window, cx),
    );
    commands.register(
        "workbench.action.files.openFile",
        "File: Open File…",
        |wb, _, window, cx| wb.prompt_and_open(true, false, window, cx),
    );
    commands.register(
        "workbench.action.files.openFolder",
        "File: Open Folder…",
        |wb, _, window, cx| wb.prompt_and_open(false, true, window, cx),
    );
    commands.register(
        "workbench.action.clearRecentFiles",
        "File: Clear Recently Opened…",
        |_, _, window, cx| clear_recent_with_prompt(window, cx).detach(),
    );
    commands.register(
        "workbench.action.closeFolder",
        "File: Close Folder",
        |wb, _, window, cx| wb.replace_with(Replacement::Welcome, window, cx),
    );
    // Picker keys only: not listed in the Command Palette.
    commands.register(
        "workbench.action.quickOpenNavigateNextInRecentFilesPicker",
        "",
        |wb, _, _, cx| wb.step_recent_picker(1, cx),
    );
    commands.register(
        "workbench.action.quickOpenNavigatePreviousInRecentFilesPicker",
        "",
        |wb, _, _, cx| wb.step_recent_picker(-1, cx),
    );
}

/// The app data directory holding the recent list, if any.
fn data_dir(cx: &App) -> Option<PathBuf> {
    cx.try_global::<RecentStore>()
        .and_then(|store| store.data_dir.clone())
}

/// Asks "Do you want to clear all recently opened folders?" and on Clear
/// empties the recent list in the background. The task ends `true` once
/// the list was cleared. Without a data directory there is no list and
/// nothing is asked.
pub(crate) fn clear_recent_with_prompt(window: &mut Window, cx: &mut App) -> Task<bool> {
    let Some(dir) = data_dir(cx) else {
        return Task::ready(false);
    };
    let answer = window.prompt(
        PromptLevel::Warning,
        CLEAR_MESSAGE,
        None,
        &CLEAR_ANSWERS[..],
        cx,
    );
    cx.spawn(async move |cx| {
        if answer.await != Ok(0) {
            return false;
        }
        let cleared = cx
            .background_spawn(async move { recent::clear(&dir) })
            .await;
        if let Err(err) = &cleared {
            tracing::warn!(%err, "could not clear the recent list");
        }
        cx.update(app_menu::refresh);
        cleared.is_ok()
    })
}

/// Forgets `path` in the stored recent list, in the background, then
/// refreshes the menu bar.
fn forget(path: PathBuf, cx: &mut App) {
    let Some(dir) = data_dir(cx) else {
        return;
    };
    cx.spawn(async move |cx| {
        let removed = cx
            .background_spawn({
                let path = path.clone();
                async move { recent::remove(&dir, &path) }
            })
            .await;
        if let Err(err) = removed {
            tracing::warn!(%err, path = %path.display(), "could not update the recent list");
        }
        cx.update(app_menu::refresh);
    })
    .detach();
}

/// Opens `path` in a new window of this process, or brings forward the
/// window showing it.
fn open_in_new_window(path: PathBuf, cx: &mut App) {
    let request = OpenRequest {
        paths: vec![path],
        target: Target::NewWindow,
        source: Source::Menu,
    };
    // Not inside this window's update: the router may update it.
    cx.defer(move |cx| {
        router::open(request, cx).ok();
    });
}

impl Workbench {
    /// The shown Open Recent picker, if any.
    pub fn recent_picker(&self) -> Option<&Entity<RecentPicker>> {
        self.recent_picker.as_ref().map(|shown| &shown.picker)
    }

    /// Shows the Open Recent picker over the recent entries, less this
    /// window's folder, once loaded in the background.
    /// When it is shown already, selects its next row.
    pub fn show_recent_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recent_picker.is_some() {
            self.step_recent_picker(1, cx);
            return;
        }
        if self.recent_picker_loading.is_some() {
            return;
        }
        self.close_modal_overlays(window, cx);
        let dir = data_dir(cx);
        let opened = self.opened_paths();
        let load = cx.spawn_in(window, async move |wb, cx| {
            let entries = match dir {
                Some(dir) => {
                    cx.background_spawn(async move {
                        recent::load(&recent::recent_path(&dir)).entries
                    })
                    .await
                }
                None => Vec::new(),
            };
            wb.update_in(cx, |wb, window, cx| {
                wb.recent_picker_loading = None;
                let entries = entries
                    .into_iter()
                    .filter(|e| !opened.contains(&e.path))
                    .collect();
                wb.open_recent_picker(entries, window, cx);
            })
            .ok();
        });
        self.recent_picker_loading = Some(load);
    }

    fn open_recent_picker(
        &mut self,
        entries: Vec<RecentEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Another overlay opened while the list loaded.
        if self.other_modal_overlay_shown() || self.locations_picker.is_some() {
            return;
        }
        let home = std::env::home_dir();
        let settings = self.settings().clone();
        let picker = cx.new(|cx| RecentPicker::new(entries, home, settings, cx));
        let events = cx.subscribe_in(&picker, window, Self::on_recent_picker_event);
        let focus_before = window.focused(cx);
        window.focus(&picker.focus_handle(cx), cx);
        self.recent_picker = Some(ShownRecentPicker {
            picker,
            _events: events,
            focus_before,
        });
        cx.notify();
    }

    /// Moves the picker's selection by `delta` rows.
    fn step_recent_picker(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(picker) = self.recent_picker() {
            picker.update(cx, |picker, cx| picker.select_next(delta, cx));
        }
    }

    /// Closes the Open Recent picker (or stops it from showing) and gives
    /// the focus back.
    pub(crate) fn close_recent_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recent_picker_loading = None;
        let Some(shown) = self.recent_picker.take() else {
            return;
        };
        let focused = window.focused(cx);
        if focused.is_none() || shown.picker.focus_handle(cx).contains_focused(window, cx) {
            match shown.focus_before {
                Some(focus) => window.focus(&focus, cx),
                None => self.focus_editor_area(window, cx),
            }
        }
        cx.notify();
    }

    fn on_recent_picker_event(
        &mut self,
        _: &Entity<RecentPicker>,
        event: &RecentPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RecentPickerEvent::Open { entry, new_window } => {
                self.close_recent_picker(window, cx);
                self.open_recent_entry(entry.clone(), *new_window, window, cx);
            }
            RecentPickerEvent::Remove(path) => forget(path.clone(), cx),
            RecentPickerEvent::Dismiss => self.close_recent_picker(window, cx),
        }
    }

    /// Opens a chosen recent entry unless it vanished since the list
    /// loaded; then says so and forgets it. A folder replaces this window
    /// (or opens beside it when `new_window`); a file opens as a tab.
    pub(crate) fn open_recent_entry(
        &mut self,
        entry: RecentEntry,
        new_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let win = window.window_handle().window_id();
        cx.spawn_in(window, async move |wb, cx| {
            let exists = {
                let entry = entry.clone();
                cx.background_spawn(async move { entry.exists() }).await
            };
            wb.update_in(cx, |wb, window, cx| {
                if !exists {
                    let path = tilde_path(&entry.path, std::env::home_dir().as_deref());
                    status_message::show(win, format!("{path} no longer exists"), cx);
                    forget(entry.path, cx);
                    return;
                }
                match (entry.kind, new_window) {
                    (_, true) => open_in_new_window(entry.path, cx),
                    (RecentKind::Folder, false) => {
                        wb.replace_with(Replacement::Path(entry.path), window, cx)
                    }
                    (RecentKind::File, false) => {
                        wb.open_path(Arc::from(entry.path), OpenMode::Pinned, cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks for a path with the system dialog and opens it
    /// ([`Workbench::open_picked_paths`]); Cancel does nothing.
    fn prompt_and_open(
        &mut self,
        files: bool,
        directories: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |wb, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Err(err)) => {
                    tracing::error!(%err, "could not show the open dialog");
                    return;
                }
                _ => return,
            };
            wb.update_in(cx, |wb, window, cx| wb.open_picked_paths(paths, window, cx))
                .ok();
        })
        .detach();
    }

    /// Opens paths picked in the Open dialog: a folder replaces this
    /// window (spec W8); files open as pinned tabs here, also outside the
    /// root.
    pub fn open_picked_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |wb, cx| {
            let (folders, files): (Vec<PathBuf>, Vec<PathBuf>) = cx
                .background_spawn(async move { paths.into_iter().partition(|p| p.is_dir()) })
                .await;
            wb.update_in(cx, |wb, window, cx| match folders.into_iter().next() {
                Some(folder) => wb.replace_with(Replacement::Path(folder), window, cx),
                None => {
                    for file in files {
                        wb.open_path(Arc::from(file.as_path()), OpenMode::Pinned, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Replaces this workbench with `replacement` in the same window
    /// (spec W8, W10): asks to save as closing the window does, saves the
    /// layout, then has the app's router swap the window's root. A path
    /// open in a window already brings that window forward instead,
    /// without asking. Cancel does nothing.
    pub fn replace_with(
        &mut self,
        replacement: Replacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Replacement::Path(path) = &replacement
            && let Some(showing) = router::window_showing(path, cx)
        {
            cx.defer(move |cx| router::activate(showing, cx));
            return;
        }
        let handle = window.window_handle();
        let confirm = self.confirm_quit(window, cx);
        cx.spawn(async move |wb, cx| {
            if !confirm.await {
                return;
            }
            if wb.update(cx, |wb, cx| wb.save_workspace_state(cx)).is_err() {
                return;
            }
            cx.update(|cx| match replacement {
                Replacement::Path(path) => {
                    let request = OpenRequest {
                        paths: vec![path],
                        target: Target::ReplaceIn(handle),
                        source: Source::Menu,
                    };
                    router::open(request, cx).ok();
                }
                Replacement::Welcome => router::open_welcome(Target::ReplaceIn(handle), cx),
            });
        })
        .detach();
    }
}
