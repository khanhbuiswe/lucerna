//! The Welcome screen: the root view of a window opened without a folder.
//! A title bar over one centered column: the logo and title, GET STARTED
//! (Open…, Open Recent…, Get Started) and up to five RECENT folders and files. The
//! window's router acts on [`WelcomeEvent`].

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, PathPromptOptions, Render, SharedString, Styled,
    Subscription, Task, Window, div, prelude::*, px, svg,
};
use lucerna_settings::{Context as WhenContext, ContextValue, Keystroke, Resolution};

use crate::app_menu;
use crate::icons;
use crate::keys;
use crate::layout::{color, title_bar};
use crate::open_commands::clear_recent_with_prompt;
use crate::recent::{self, RecentEntry, RecentKind, RecentStore, display_parts};
use crate::ui::{
    CLOSE_ICON, GRID, UI_FONT_FAMILY, UI_FONT_SIZE, UI_FONT_SIZE_SMALL, UI_FONT_SIZE_STATUS,
    icon_button, list_button, popover_overlay, section_header, tooltip, warning_row,
};
use crate::workspace_state::tilde_path;
use crate::{AppState, RecentPicker, RecentPickerEvent, WindowSettings};

const OPEN_FILE_FOLDER: &str = "workbench.action.files.openFileFolder";
const OPEN_FILE: &str = "workbench.action.files.openFile";
const OPEN_FOLDER: &str = "workbench.action.files.openFolder";
const OPEN_RECENT: &str = "workbench.action.openRecent";
const CLEAR_RECENT: &str = "workbench.action.clearRecentFiles";
const PICKER_NEXT: &str = "workbench.action.quickOpenNavigateNextInRecentFilesPicker";
const PICKER_PREVIOUS: &str = "workbench.action.quickOpenNavigatePreviousInRecentFilesPicker";
const OPEN_ONBOARDING: &str = "lucerna.openOnboarding";
const ABOUT: &str = "workbench.action.showAboutDialog";
const QUIT: &str = "workbench.action.quit";
const NEW_WINDOW: &str = crate::open_commands::NEW_WINDOW;

/// The commands the Welcome screen runs; keys bound to any other command
/// do nothing here.
pub const WELCOME_COMMANDS: &[&str] = &[
    NEW_WINDOW,
    OPEN_FILE_FOLDER,
    OPEN_FILE,
    OPEN_FOLDER,
    OPEN_RECENT,
    CLEAR_RECENT,
    PICKER_NEXT,
    PICKER_PREVIOUS,
    OPEN_ONBOARDING,
    ABOUT,
    QUIT,
];

/// How many recent entries the page lists; "More…" opens the rest in the
/// picker.
const RECENT_ROWS: usize = 5;

/// The content column's widest.
const COLUMN_MAX_WIDTH: f32 = 512.;
const LOGO_SIZE: f32 = 45.;
const TITLE_FONT_SIZE: f32 = 20.;

/// What the window's router does for the Welcome screen.
#[derive(Debug, Clone, PartialEq)]
pub enum WelcomeEvent {
    /// Open this folder or file: in this window, or a new one when
    /// `new_window`.
    Open {
        path: PathBuf,
        new_window: bool,
    },
    /// Show the Get Started page in place of Welcome.
    OpenOnboarding,
    Quit,
}

/// One button of the page, in focus order.
#[derive(Clone, Copy, PartialEq)]
enum Button {
    /// Runs this command: a GET STARTED button.
    Command(&'static str),
    /// The recent entry at this index.
    Recent(usize),
    More,
}

/// A GET STARTED button: debug selector, icon, label, and the command it
/// runs, whose key it shows.
struct StartButton {
    selector: &'static str,
    icon: &'static [u8],
    label: &'static str,
    command: &'static str,
}

/// The GET STARTED buttons, in order.
#[cfg(target_os = "macos")]
const START_BUTTONS: &[StartButton] = &[
    StartButton {
        selector: "welcome-open",
        icon: icons::FOLDER_OPEN,
        label: "Open…",
        command: OPEN_FILE_FOLDER,
    },
    OPEN_RECENT_BUTTON,
    GET_STARTED_BUTTON,
];
#[cfg(not(target_os = "macos"))]
const START_BUTTONS: &[StartButton] = &[
    StartButton {
        selector: "welcome-open-file",
        icon: icons::FILE,
        label: "Open File…",
        command: OPEN_FILE,
    },
    StartButton {
        selector: "welcome-open-folder",
        icon: icons::FOLDER_OPEN,
        label: "Open Folder…",
        command: OPEN_FOLDER,
    },
    OPEN_RECENT_BUTTON,
    GET_STARTED_BUTTON,
];
const OPEN_RECENT_BUTTON: StartButton = StartButton {
    selector: "welcome-open-recent",
    icon: icons::HISTORY,
    label: "Open Recent…",
    command: OPEN_RECENT,
};
const GET_STARTED_BUTTON: StartButton = StartButton {
    selector: "welcome-get-started",
    icon: icons::SETTINGS,
    label: "Get Started",
    command: OPEN_ONBOARDING,
};

pub struct WelcomeView {
    focus: FocusHandle,
    data_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    /// The recent entries that existed when loaded, newest first; `None`
    /// while loading.
    recent: Option<Vec<RecentEntry>>,
    /// Index into [`Self::buttons`] of the focused button.
    focused: usize,
    /// "‹path› no longer exists", after a click on a vanished entry.
    warning: Option<String>,
    picker: Option<(Entity<RecentPicker>, Subscription)>,
    pending_chord: Vec<Keystroke>,
    /// Whether a press began on the title bar, for its drag.
    title_bar_pressed: Rc<Cell<bool>>,
    /// This window's settings; it has no folder, so no workspace layer.
    settings: Entity<WindowSettings>,
    _load: Task<()>,
}

impl EventEmitter<WelcomeEvent> for WelcomeView {}

impl WelcomeView {
    /// Focuses the page and starts loading the recent list in the
    /// background.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::config_files::follow_system_appearance(window, cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let data_dir = cx
            .try_global::<RecentStore>()
            .and_then(|store| store.data_dir.clone());
        let load = {
            let data_dir = data_dir.clone();
            cx.spawn(async move |this, cx| {
                let entries = match data_dir {
                    Some(dir) => {
                        cx.background_spawn(async move {
                            recent::load(&recent::recent_path(&dir)).existing()
                        })
                        .await
                    }
                    None => Vec::new(),
                };
                this.update(cx, |view, cx| {
                    view.recent = Some(entries);
                    cx.notify();
                })
                .ok();
            })
        };
        WelcomeView {
            focus,
            data_dir,
            home: std::env::home_dir(),
            recent: None,
            focused: 0,
            warning: None,
            picker: None,
            pending_chord: Vec::new(),
            title_bar_pressed: Rc::default(),
            // A window without a folder: the app's settings alone.
            settings: cx.new(|cx| WindowSettings::new(None, cx)),
            _load: load,
        }
    }

    /// The recent entries, newest first; `None` while loading.
    pub fn recent(&self) -> Option<&[RecentEntry]> {
        self.recent.as_deref()
    }

    pub fn recent_picker(&self) -> Option<&Entity<RecentPicker>> {
        self.picker.as_ref().map(|(picker, _)| picker)
    }

    /// "Welcome back to Lucerna" once recent entries are known to exist.
    pub fn title(&self) -> &'static str {
        if self.recent.as_ref().is_some_and(|r| !r.is_empty()) {
            "Welcome back to Lucerna"
        } else {
            "Welcome to Lucerna"
        }
    }

    /// The focused button's place in focus order: the open buttons, Open
    /// Recent…, the recent rows, then More….
    pub fn focused_index(&self) -> usize {
        self.focused
    }

    /// The warning about a vanished recent entry, if one shows.
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// Opens the Open Recent picker over the page with every recent entry;
    /// when it is open already, selects its next row.
    pub fn show_recent_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((picker, _)) = &self.picker {
            picker.update(cx, |picker, cx| picker.select_next(1, cx));
            return;
        }
        let entries = self.recent.clone().unwrap_or_default();
        let home = self.home.clone();
        let settings = self.settings.clone();
        let picker = cx.new(|cx| RecentPicker::new(entries, home, settings, cx));
        let events = cx.subscribe_in(&picker, window, Self::on_picker_event);
        window.focus(&picker.focus_handle(cx), cx);
        self.picker = Some((picker, events));
        cx.notify();
    }

    /// Runs one of [`WELCOME_COMMANDS`]; `false` for any other command.
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match id {
            OPEN_FILE_FOLDER => self.prompt_for_path(true, true, cx),
            OPEN_FILE => self.prompt_for_path(true, false, cx),
            OPEN_FOLDER => self.prompt_for_path(false, true, cx),
            OPEN_RECENT => self.show_recent_picker(window, cx),
            PICKER_NEXT => self.step_picker(1, cx),
            PICKER_PREVIOUS => self.step_picker(-1, cx),
            CLEAR_RECENT => self.clear_recent(window, cx),
            OPEN_ONBOARDING => cx.emit(WelcomeEvent::OpenOnboarding),
            ABOUT => crate::about::show(window, cx),
            QUIT => cx.emit(WelcomeEvent::Quit),
            // Not inside this window's update: the router opens a window.
            NEW_WINDOW => {
                cx.defer(|cx| crate::router::open_welcome(crate::router::Target::NewWindow, cx))
            }
            _ => return false,
        }
        true
    }

    /// Moves the picker's selection by `delta` rows.
    fn step_picker(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some((picker, _)) = &self.picker {
            picker.update(cx, |picker, cx| picker.select_next(delta, cx));
        }
    }

    /// Asks before clearing the recent list; once cleared, the RECENT
    /// section goes.
    fn clear_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cleared = clear_recent_with_prompt(window, cx);
        cx.spawn(async move |this, cx| {
            if cleared.await {
                this.update(cx, |view, cx| {
                    view.recent = Some(Vec::new());
                    view.focused = view.focused.min(view.buttons().len() - 1);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Asks for a path to open; Cancel leaves the page as it is.
    fn prompt_for_path(&mut self, files: bool, directories: bool, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |this, cx| {
            let path = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(err)) => {
                    tracing::error!(%err, "could not show the open dialog");
                    None
                }
                _ => None,
            };
            if let Some(path) = path {
                this.update(cx, |_, cx| {
                    cx.emit(WelcomeEvent::Open {
                        path,
                        new_window: false,
                    })
                })
                .ok();
            }
        })
        .detach();
    }

    /// The buttons in focus order.
    fn buttons(&self) -> Vec<Button> {
        let recent = self.recent.as_ref().map_or(0, Vec::len);
        START_BUTTONS
            .iter()
            .map(|b| Button::Command(b.command))
            .chain((0..recent.min(RECENT_ROWS)).map(Button::Recent))
            .chain((recent > RECENT_ROWS).then_some(Button::More))
            .collect()
    }

    fn is_focused(&self, button: Button) -> bool {
        self.buttons().get(self.focused) == Some(&button)
    }

    fn move_focus(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.buttons().len() as isize;
        self.focused = (self.focused as isize + delta).rem_euclid(count) as usize;
        cx.notify();
    }

    fn press(
        &mut self,
        button: Button,
        new_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match button {
            Button::Command(command) => {
                self.run_command(command, window, cx);
            }
            Button::More => self.show_recent_picker(window, cx),
            Button::Recent(index) => {
                if let Some(entry) = self.recent.as_ref().and_then(|r| r.get(index)) {
                    self.open_entry(entry.clone(), new_window, cx);
                }
            }
        }
    }

    /// Opens `entry` unless it vanished since the list loaded; then drops
    /// it and shows a warning instead.
    pub(crate) fn open_entry(
        &mut self,
        entry: RecentEntry,
        new_window: bool,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let exists = {
                let entry = entry.clone();
                cx.background_spawn(async move { entry.exists() }).await
            };
            this.update(cx, |view, cx| {
                if exists {
                    cx.emit(WelcomeEvent::Open {
                        path: entry.path,
                        new_window,
                    });
                } else {
                    let path = tilde_path(&entry.path, view.home.as_deref());
                    view.warning = Some(format!("{path} no longer exists"));
                    view.forget(entry.path, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Drops `path` from the page and, in the background, from the stored
    /// list; then rebuilds the menu bar's Open Recent.
    fn forget(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(recent) = &mut self.recent {
            recent.retain(|e| e.path != path);
        }
        self.focused = self.focused.min(self.buttons().len() - 1);
        cx.notify();
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        cx.spawn(async move |_, cx| {
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

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn on_picker_event(
        &mut self,
        _: &Entity<RecentPicker>,
        event: &RecentPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RecentPickerEvent::Open { entry, new_window } => {
                self.close_picker(window, cx);
                self.open_entry(entry.clone(), *new_window, cx);
            }
            RecentPickerEvent::Remove(path) => self.forget(path.clone(), cx),
            RecentPickerEvent::Dismiss => self.close_picker(window, cx),
        }
    }

    /// Runs keys bound to a Welcome command, then moves focus and presses
    /// buttons. Keys for anything else pass on, to the picker when open.
    fn dispatch_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = keys::to_keystroke(ev);
        let mut ctx = WhenContext::default();
        ctx.set("inQuickOpen", ContextValue::Bool(self.picker.is_some()));
        ctx.set("inRecentPicker", ContextValue::Bool(self.picker.is_some()));
        crate::keys::set_platform_flags(&mut ctx);
        let keymap = &cx.global::<AppState>().keymap;
        match keys::resolve_chord(&mut self.pending_chord, &stroke, &ctx, keymap) {
            Resolution::Command { command, .. } => {
                if WELCOME_COMMANDS.contains(&command.as_str())
                    && self.run_command(&command, window, cx)
                {
                    cx.stop_propagation();
                    return;
                }
            }
            Resolution::Pending => {
                cx.stop_propagation();
                return;
            }
            Resolution::NoMatch => {}
        }
        if self.picker.is_none() && self.page_key(ev, &stroke, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Arrows and Tab move focus, Enter presses (⌘Enter in a new window),
    /// ⌘1…⌘5 open a recent entry.
    fn page_key(
        &mut self,
        ev: &KeyDownEvent,
        stroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let m = &ev.keystroke.modifiers;
        let recent = self.recent.as_ref().map_or(0, Vec::len);
        if let Some(index) = (0..recent.min(RECENT_ROWS)).find(|&i| *stroke == recent_keystroke(i))
        {
            self.press(Button::Recent(index), false, window, cx);
            return true;
        }
        let plain = !(m.control || m.alt || m.platform || m.function);
        match ev.keystroke.key.as_str() {
            "up" if plain && !m.shift => self.move_focus(-1, cx),
            "down" if plain && !m.shift => self.move_focus(1, cx),
            "tab" if plain => self.move_focus(if m.shift { -1 } else { 1 }, cx),
            "enter" if !m.shift && !m.alt => {
                let new_window = m.secondary(cx);
                if !new_window && !plain {
                    return false;
                }
                if let Some(button) = self.buttons().get(self.focused).copied() {
                    self.press(button, new_window, window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3. * GRID))
            .child(
                svg()
                    .data(icons::LUCERNA_MARK)
                    .flex_none()
                    .size(px(LOGO_SIZE))
                    .text_color(color(ui.text)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .debug_selector(|| "welcome-title".into())
                            .text_size(px(TITLE_FONT_SIZE))
                            .child(self.title()),
                    )
                    .child(
                        div()
                            .text_size(px(UI_FONT_SIZE_SMALL))
                            .text_color(color(ui.text_muted))
                            .child("A minimal code editor"),
                    ),
            )
            .into_any_element()
    }

    fn render_get_started(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = START_BUTTONS
            .iter()
            .map(|b| {
                let button = Button::Command(b.command);
                let selector = b.selector;
                let keys = cx.global::<AppState>().keymap.label_for(b.command);
                list_button(
                    selector,
                    b.icon,
                    b.label.into(),
                    keys,
                    self.is_focused(button),
                    cx,
                )
                .debug_selector(move || selector.into())
                .on_click(cx.listener(move |view, ev: &ClickEvent, window, cx| {
                    view.click(button, ev, window, cx)
                }))
                .into_any_element()
            })
            .collect();
        section(
            section_header("Get Started", self.settings.read(cx).view(cx), cx),
            rows,
        )
        .into_any_element()
    }

    fn render_recent(&self, recent: &[RecentEntry], cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = recent
            .iter()
            .take(RECENT_ROWS)
            .enumerate()
            .map(|(index, entry)| self.render_recent_row(index, entry, cx))
            .collect();
        if recent.len() > RECENT_ROWS {
            rows.push(
                list_button(
                    "welcome-more",
                    icons::CHEVRON_RIGHT,
                    "More…".into(),
                    None,
                    self.is_focused(Button::More),
                    cx,
                )
                .debug_selector(|| "welcome-more".into())
                .on_click(cx.listener(|view, ev: &ClickEvent, window, cx| {
                    view.click(Button::More, ev, window, cx)
                }))
                .into_any_element(),
            );
        }
        section(
            section_header("Recent", self.settings.read(cx).view(cx), cx),
            rows,
        )
        .into_any_element()
    }

    /// Name, folder, shortcut, then a × that shows on hover or focus.
    fn render_recent_row(
        &self,
        index: usize,
        entry: &RecentEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let (name, parent) = display_parts(entry, self.home.as_deref());
        let icon = match entry.kind {
            RecentKind::Folder => icons::FOLDER,
            RecentKind::File => icons::FILE,
        };
        let focused = self.is_focused(Button::Recent(index));
        let group = SharedString::from(format!("welcome-recent-group-{index}"));
        let text_muted = color(ui.text_muted);
        let remove = icon_button(("welcome-recent-remove", index), CLOSE_ICON, false, cx)
            .debug_selector(move || format!("welcome-recent-remove-{index}"))
            .tooltip(tooltip("Remove from Recently Opened"))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
                if let Some(entry) = view.recent.as_ref().and_then(|r| r.get(index)) {
                    let path = entry.path.clone();
                    view.forget(path, cx);
                }
            }));
        list_button(
            ("welcome-recent", index),
            icon,
            name.into(),
            None,
            focused,
            cx,
        )
        .debug_selector(move || format!("welcome-recent-{index}"))
        .group(group.clone())
        .on_click(cx.listener(move |view, ev: &ClickEvent, window, cx| {
            view.click(Button::Recent(index), ev, window, cx)
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(text_muted)
                .child(parent),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(UI_FONT_SIZE_STATUS))
                .text_color(text_muted)
                .child(recent_keystroke(index).label()),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .when(!focused, |d| {
                    d.invisible().group_hover(group, |s| s.visible())
                })
                .child(remove),
        )
        .into_any_element()
    }

    /// A click focuses the button and presses it; with ⌘ held, a recent
    /// entry opens in a new window.
    fn click(
        &mut self,
        button: Button,
        ev: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.buttons().iter().position(|b| *b == button) {
            self.focused = index;
        }
        let new_window = ev.modifiers().secondary(cx);
        self.press(button, new_window, window, cx);
        cx.notify();
    }
}

/// The key that opens the recent entry at `index` (0-based): ⌘1…⌘5 on
/// macOS, Ctrl+1…Ctrl+5 elsewhere.
fn recent_keystroke(index: usize) -> Keystroke {
    let mac = cfg!(target_os = "macos");
    Keystroke {
        cmd: mac,
        ctrl: !mac,
        alt: false,
        shift: false,
        key: (index + 1).to_string(),
    }
}

/// A section: its header, then its rows.
fn section(header: gpui::Div, rows: Vec<AnyElement>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(GRID))
        .child(header.mb(px(GRID)))
        .children(rows)
}

impl Focusable for WelcomeView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let (background, text) = (color(ui.editor_background), color(ui.text));
        let title_bar = title_bar::render_plain("Lucerna", self.title_bar_pressed.clone(), ui);
        let warning = self
            .warning
            .clone()
            .map(|text| warning_row("welcome-warning".into(), text, cx));
        let header = self.render_header(cx);
        let get_started = self.render_get_started(cx);
        let recent = self
            .recent
            .clone()
            .filter(|r| !r.is_empty())
            .map(|r| self.render_recent(&r, cx));
        div()
            .id("welcome")
            .debug_selector(|| "welcome".into())
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::dispatch_key))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(text)
            .font_family(UI_FONT_FAMILY)
            .text_size(px(UI_FONT_SIZE))
            .child(title_bar)
            .child(
                div()
                    .id("welcome-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .items_center()
                            .min_h_full()
                            .p(px(8. * GRID))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6. * GRID))
                                    .w_full()
                                    .max_w(px(COLUMN_MAX_WIDTH))
                                    .children(warning)
                                    .child(header)
                                    .child(get_started)
                                    .children(recent),
                            ),
                    ),
            )
            .children(
                self.recent_picker()
                    .map(|picker| popover_overlay(picker.clone())),
            )
    }
}
