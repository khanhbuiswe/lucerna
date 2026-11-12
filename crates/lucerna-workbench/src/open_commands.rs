//! Opening folders and files from the workbench: Open Recent (`ctrl+r`),
//! Open…, Open File…, Open Folder…, Close Folder and Clear Recently
//! Opened. A process shows one window, so another folder opens in a new
//! `luc` process: in place of this window (spec W8) after the save prompt,
//! or beside it. A file opens as a tab here, as VS Code does.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, PathPromptOptions, PromptLevel, Subscription,
    Task, Window, prelude::*,
};

use crate::app_menu;
use crate::process::{self, SpawnRequest};
use crate::recent::{self, RecentEntry, RecentKind, RecentStore};
use crate::status_message;
use crate::workspace_state::tilde_path;
use crate::{CommandRegistry, OpenMode, RecentPicker, RecentPickerEvent, Workbench};

const CLEAR_MESSAGE: &str = "Do you want to clear all recently opened folders?";
const CLEAR_ANSWERS: [&str; 2] = ["Clear", "Cancel"];

/// The shown Open Recent picker.
pub(crate) struct ShownRecentPicker {
    picker: Entity<RecentPicker>,
    _events: Subscription,
    focus_before: Option<FocusHandle>,
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
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
        |wb, _, window, cx| wb.replace_with(SpawnRequest::Welcome, window, cx),
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

/// Starts the process `request` describes; on failure says why in the
/// status bar.
fn spawn_or_report(request: &SpawnRequest, cx: &mut App) -> bool {
    match process::spawn(request, cx) {
        Ok(()) => true,
        Err(err) => {
            let what = match request {
                SpawnRequest::Path(path) => tilde_path(path, std::env::home_dir().as_deref()),
                SpawnRequest::Welcome => "Lucerna".into(),
            };
            tracing::error!(%err, "could not start a new process");
            status_message::show(format!("Could not open {what}: {err}"), cx);
            false
        }
    }
}

impl Workbench {
    /// The shown Open Recent picker, if any.
    pub fn recent_picker(&self) -> Option<&Entity<RecentPicker>> {
        self.recent_picker.as_ref().map(|shown| &shown.picker)
    }

    /// Shows the Open Recent picker over the recent entries that still
    /// exist, less this window's folder, once loaded in the background.
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
                        recent::load(&recent::recent_path(&dir)).existing()
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
        let picker = cx.new(|cx| RecentPicker::new(entries, home, cx));
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
        cx.spawn_in(window, async move |wb, cx| {
            let exists = {
                let entry = entry.clone();
                cx.background_spawn(async move { entry.exists() }).await
            };
            wb.update_in(cx, |wb, window, cx| {
                if !exists {
                    let path = tilde_path(&entry.path, std::env::home_dir().as_deref());
                    status_message::show(format!("{path} no longer exists"), cx);
                    forget(entry.path, cx);
                    return;
                }
                let request = SpawnRequest::Path(entry.path.clone());
                match (entry.kind, new_window) {
                    (_, true) => {
                        spawn_or_report(&request, cx);
                    }
                    (RecentKind::Folder, false) => wb.replace_with(request, window, cx),
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
                Some(folder) => wb.replace_with(SpawnRequest::Path(folder), window, cx),
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

    /// Replaces this window with a new process showing `request` (spec
    /// W8, W10): asks to save as closing the window does, saves the
    /// layout, starts the process, then closes the window. Cancel does
    /// nothing; a process that does not start keeps the window and says
    /// why in the status bar.
    pub(crate) fn replace_with(
        &mut self,
        request: SpawnRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let confirm = self.confirm_quit(window, cx);
        cx.spawn_in(window, async move |wb, cx| {
            if !confirm.await {
                return;
            }
            wb.update_in(cx, |wb, window, cx| {
                wb.save_workspace_state(cx);
                if spawn_or_report(&request, cx) {
                    window.remove_window();
                }
            })
            .ok();
        })
        .detach();
    }
}
