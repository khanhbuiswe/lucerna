//! The Open Recent picker: the recent folders and files in Quick Open's
//! list, fuzzy-filtered on name and folder. Folders come before files,
//! under headers when both exist. Enter opens the row, `⌘Enter` opens it
//! in a new window, and `ctrl-r` / `ctrl-shift-r` step through the rows.
//! The Welcome screen and the workbench host it and act on its events.

use std::path::PathBuf;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Modifiers, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    Styled, Window, div, prelude::*, px, svg,
};

use crate::icons;
use crate::layout::color;
use crate::quick_open::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, wrap_index,
};
use crate::recent::{RecentEntry, RecentKind, display_parts};
use crate::text_input::TextInput;
use crate::ui::{
    CLOSE_ICON, FILE_ICON_SIZE, GRID, UI_FONT_SIZE, UI_FONT_SIZE_SMALL, icon_button, popover,
    section_header, tooltip,
};
use crate::{AppState, WindowSettings};

/// The search box's hint, naming the key that opens a new window.
#[cfg(target_os = "macos")]
const PLACEHOLDER: &str = "Select to open (⌘Enter for a new window)";
#[cfg(not(target_os = "macos"))]
const PLACEHOLDER: &str = "Select to open (Ctrl+Enter for a new window)";

/// What the host does as the picker is used.
#[derive(Debug, Clone, PartialEq)]
pub enum RecentPickerEvent {
    /// Enter, a click or `⌘Enter` chose this entry.
    Open {
        entry: RecentEntry,
        new_window: bool,
    },
    /// The × button asked to forget this path. The row is gone from the
    /// picker; the host updates the stored list.
    Remove(PathBuf),
    Dismiss,
}

pub struct RecentPicker {
    input: Entity<TextInput>,
    /// Folders first, then files, each in the order given.
    entries: Vec<RecentEntry>,
    /// `"{name} {parent}"` for each of `entries`: what the query matches.
    texts: Vec<String>,
    /// The name's length in chars within the matching `texts` item.
    name_chars: Vec<usize>,
    /// Matching entries: index into `entries` and matched char positions.
    rows: Vec<(usize, Vec<u32>)>,
    query: String,
    selected: usize,
    scroll: ScrollHandle,
    /// The window's settings: the section headers use its editor font.
    settings: Entity<WindowSettings>,
}

impl EventEmitter<RecentPickerEvent> for RecentPicker {}

impl RecentPicker {
    /// Lists `entries` (newest first); `home` shortens folders to `~`.
    /// `settings` are the window's.
    pub fn new(
        entries: Vec<RecentEntry>,
        home: Option<PathBuf>,
        settings: Entity<WindowSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder(PLACEHOLDER);
            input.on_change(move |text, _, cx| {
                this.update(cx, |picker, cx| picker.query_changed(text, cx))
                    .ok();
            });
            input
        });
        let (folders, files): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .partition(|e| e.kind == RecentKind::Folder);
        let entries: Vec<RecentEntry> = folders.into_iter().chain(files).collect();
        let mut texts = Vec::with_capacity(entries.len());
        let mut name_chars = Vec::with_capacity(entries.len());
        for entry in &entries {
            let (name, parent) = display_parts(entry, home.as_deref());
            name_chars.push(name.chars().count());
            texts.push(format!("{name} {parent}"));
        }
        let rows = fuzzy_filter(texts.iter().map(String::as_str), "");
        RecentPicker {
            input,
            entries,
            texts,
            name_chars,
            rows,
            query: String::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            settings,
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The rows shown, in order, as `"{name} {parent}"`.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(i, _)| self.texts[*i].clone())
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Moves the selection by `delta` rows, wrapping at both ends.
    pub fn select_next(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.select(wrap_index(self.selected, self.rows.len(), delta), cx);
    }

    /// Opens the selected row, in a new window when `new_window`.
    pub fn confirm(&mut self, new_window: bool, cx: &mut Context<Self>) {
        self.open(self.selected, new_window, cx);
    }

    /// Forgets the row: emits [`RecentPickerEvent::Remove`] and drops it
    /// from the list. The picker stays open.
    pub fn remove_at(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(&(index, _)) = self.rows.get(row) else {
            return;
        };
        let entry = self.entries.remove(index);
        self.texts.remove(index);
        self.name_chars.remove(index);
        self.refilter();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        cx.emit(RecentPickerEvent::Remove(entry.path));
        cx.notify();
    }

    fn open(&mut self, row: usize, new_window: bool, cx: &mut Context<Self>) {
        if let Some(&(index, _)) = self.rows.get(row) {
            cx.emit(RecentPickerEvent::Open {
                entry: self.entries[index].clone(),
                new_window,
            });
        }
    }

    fn refilter(&mut self) {
        self.rows = fuzzy_filter(self.texts.iter().map(String::as_str), &self.query);
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.query = text.trim().to_string();
        self.refilter();
        self.select(0, cx);
    }

    fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        self.selected = row;
        self.scroll.scroll_to_item(self.child_index(row));
        cx.notify();
    }

    /// Whether the FOLDERS and FILES headers show: both kinds exist and
    /// nothing is being filtered.
    fn show_headers(&self) -> bool {
        let folders = self.entries.iter().any(|e| e.kind == RecentKind::Folder);
        let files = self.entries.iter().any(|e| e.kind == RecentKind::File);
        self.query.is_empty() && folders && files
    }

    /// Where `row` sits among the list's children, which headers add to.
    fn child_index(&self, row: usize) -> usize {
        if !self.show_headers() {
            return row;
        }
        let is_file = |r: usize| self.entries[self.rows[r].0].kind == RecentKind::File;
        let first_file = (0..self.rows.len()).find(|&r| is_file(r));
        1 + row + usize::from(first_file.is_some_and(|f| row >= f))
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        // ⌘Enter on macOS, Ctrl+Enter elsewhere: a new window.
        let new_window = *m == Modifiers::secondary_key(cx);
        match ev.keystroke.key.as_str() {
            "r" if m.control && !m.alt && !m.platform => {
                self.select_next(if m.shift { -1 } else { 1 }, cx)
            }
            "enter" if new_window => self.confirm(true, cx),
            _ if m.control || m.alt || m.shift || m.platform => return,
            "enter" => self.confirm(false, cx),
            "up" => self.select_next(-1, cx),
            "down" => self.select_next(1, cx),
            // Unless a key binding closed the picker first.
            "escape" => cx.emit(RecentPickerEvent::Dismiss),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let (index, positions) = &self.rows[row];
        let entry = &self.entries[*index];
        let ui = &cx.global::<AppState>().theme.ui;
        let icon = match entry.kind {
            RecentKind::Folder => icons::FOLDER,
            RecentKind::File => icons::FILE,
        };
        // The text is "{name} {parent}": split the matches at the space.
        let split = self.name_chars[*index] as u32;
        let (name_hits, parent_hits): (Vec<u32>, Vec<u32>) = {
            let (a, b): (Vec<_>, Vec<_>) = positions.iter().partition(|&&p| p < split);
            (
                a,
                b.into_iter().map(|p| p.saturating_sub(split + 1)).collect(),
            )
        };
        let text = &self.texts[*index];
        let name: SharedString = text.chars().take(split as usize).collect::<String>().into();
        let parent: SharedString = text
            .chars()
            .skip(split as usize + 1)
            .collect::<String>()
            .into();
        let group = SharedString::from(format!("recent-picker-group-{row}"));
        let button = |id: &'static str, svg_data: &'static [u8], tip: &'static str| {
            icon_button((id, row), svg_data, false, cx)
                .debug_selector(move || format!("{id}-{row}"))
                .tooltip(tooltip(tip))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
        };
        picker_row("recent-picker-row", row, row == self.selected, cx)
            .group(group.clone())
            .on_click(cx.listener(move |picker, ev: &gpui::ClickEvent, _, cx| {
                picker.select(row, cx);
                picker.open(row, ev.modifiers().secondary(cx), cx);
            }))
            .gap(px(2. * GRID))
            .child(
                svg()
                    .data(icon)
                    .size(px(FILE_ICON_SIZE))
                    .flex_none()
                    .text_color(color(ui.icon_muted)),
            )
            .child(
                div()
                    .flex_none()
                    .max_w(px(80. * GRID))
                    .truncate()
                    .text_size(px(UI_FONT_SIZE))
                    .child(highlighted(name, &name_hits, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .text_color(color(ui.text_muted))
                    .child(highlighted(parent, &parent_hits, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .child(
                        button(
                            "recent-picker-new-window",
                            icons::ARROW_UP_RIGHT,
                            "Open in New Window (⌘Enter)",
                        )
                        .on_click(cx.listener(
                            move |picker, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                picker.open(row, true, cx);
                            },
                        )),
                    )
                    .child(
                        button(
                            "recent-picker-remove",
                            CLOSE_ICON,
                            "Remove from Recently Opened",
                        )
                        .on_click(cx.listener(
                            move |picker, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                picker.remove_at(row, cx);
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    fn render_header(&self, title: &'static str, selector: &'static str, cx: &App) -> AnyElement {
        section_header(title, self.settings.read(cx).view(cx), cx)
            .debug_selector(|| selector.to_string())
            .px(px(2. * GRID))
            .pt(px(2. * GRID))
            .pb(px(GRID))
            .into_any_element()
    }
}

impl Focusable for RecentPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for RecentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut children: Vec<AnyElement> = Vec::new();
        let headers = self.show_headers();
        let mut in_files = false;
        for row in 0..self.rows.len() {
            let is_file = self.entries[self.rows[row].0].kind == RecentKind::File;
            if headers && row == 0 {
                children.push(self.render_header("Folders", "recent-picker-header-folders", cx));
            }
            if headers && is_file && !in_files {
                in_files = true;
                children.push(self.render_header("Files", "recent-picker-header-files", cx));
            }
            children.push(self.render_row(row, cx));
        }
        let ui = &cx.global::<AppState>().theme.ui;
        let empty = self.rows.is_empty().then(|| {
            let message = if self.entries.is_empty() {
                "No recently opened folders"
            } else {
                "No matching results"
            };
            div()
                .debug_selector(|| "recent-picker-empty".to_string())
                .px(px(3. * GRID))
                .py(px(3. * GRID))
                .text_size(px(UI_FONT_SIZE))
                .text_color(color(ui.text_muted))
                .child(message)
        });
        popover(cx)
            .id("recent-picker")
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(RecentPickerEvent::Dismiss)))
            .child(picker_input_row(&self.input, cx))
            .when(!children.is_empty(), |p| {
                p.child(picker_list("recent-picker-list", &self.scroll).children(children))
            })
            .children(empty)
    }
}
