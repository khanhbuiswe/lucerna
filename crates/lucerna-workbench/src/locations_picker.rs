//! The locations picker (spec §7.4): several definitions, references…
//! listed by file on the left, with a read-only preview of the selected
//! one on the right, like Zed's locations modal. It is drawn with the
//! shared picker pieces of Quick Open.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    HighlightStyle, IntoElement, KeyDownEvent, ParentElement, Render, ScrollStrategy, SharedString,
    StyledText, Subscription, UniformListScrollHandle, WeakEntity, Window, div, prelude::*, px,
    relative, uniform_list,
};
use lucerna_lsp::locations::ResolvedLocation;

use crate::code_view::CodeView;
use crate::documents::Documents;
use crate::fonts::editor_font_family;
use crate::goto::{GotoKind, open_location};
use crate::layout::color;
use crate::quick_open::picker::{
    PICKER_ROW_HEIGHT, fuzzy_filter, match_color, picker_input_row, picker_row,
};
use crate::text_input::TextInput;
use crate::ui::{GRID, UI_FONT_SIZE, UI_FONT_SIZE_SMALL, file_icon, popover};
use crate::{AppState, Workbench};

/// The widest the picker gets, and the share of the window it takes.
const MAX_WIDTH: f32 = 1100.;
const WIDTH_SHARE: f32 = 0.9;
/// The tallest the picker gets, and the share of the window it takes.
const MAX_HEIGHT: f32 = 560.;
const HEIGHT_SHARE: f32 = 0.7;
/// The list's share of the width; the preview takes the rest.
const LIST_SHARE: f32 = 0.4;
/// A code font char's width, in ems: sizes the line number column.
const CODE_CHAR_EM: f32 = 0.6;
/// The most chars of a line a row shows and the filter reads, ellipses
/// included: a minified line can be megabytes.
pub const EXCERPT_CHARS: usize = 200;
/// At most this many chars before the range when a line is cut.
const EXCERPT_BEFORE: usize = 40;

/// What the workbench does as the picker is used.
#[derive(Debug, Clone, PartialEq)]
pub enum LocationsPickerEvent {
    /// Open this location; `aside` in the group to the right.
    Open {
        location: ResolvedLocation,
        aside: bool,
    },
    Dismiss,
}

/// The locations of one file.
struct FileGroup {
    name: SharedString,
    /// The directory, relative to the root when inside it.
    dir: SharedString,
    path: Arc<Path>,
}

/// The part of a location's line a row shows: leading whitespace
/// dropped, cut to [`EXCERPT_CHARS`] around the range.
struct Excerpt {
    text: SharedString,
    /// The range's bytes on its first row, in `text`.
    range: Range<usize>,
}

enum Row {
    /// A file's header: not selectable.
    Header { file: usize, count: usize },
    /// A location; `positions` are matched char indices into its
    /// excerpt.
    Location { index: usize, positions: Vec<u32> },
}

pub struct LocationsPicker {
    input: Entity<TextInput>,
    documents: WeakEntity<Documents>,
    /// Sorted: the current file first, then by path; by place within one.
    locations: Vec<ResolvedLocation>,
    /// The file group of each location.
    location_files: Vec<usize>,
    /// What each location's row shows.
    excerpts: Vec<Excerpt>,
    /// What the filter matches for each location: "file name excerpt".
    haystacks: Vec<String>,
    files: Vec<FileGroup>,
    rows: Vec<Row>,
    /// The selected row, always a location.
    selected: Option<usize>,
    /// Enter and a double click open in the group to the right.
    aside: bool,
    scroll: UniformListScrollHandle,
    /// The selected location's file, read-only; dropped with the picker.
    preview: Option<Entity<CodeView>>,
    /// Centres the preview once its text has loaded.
    _preview_loaded: Option<Subscription>,
    /// Digits of the largest line number.
    digits: usize,
}

impl EventEmitter<LocationsPickerEvent> for LocationsPicker {}

/// The shown picker, its events, and where focus goes back to.
pub(crate) struct ShownLocationsPicker {
    picker: Entity<LocationsPicker>,
    focus_before: Option<FocusHandle>,
    _events: Subscription,
}

impl LocationsPicker {
    /// Shows `locations` found by `kind` over the workbench, replacing a
    /// picker already shown, and focuses its input. `current` (the file
    /// the lookup came from) is listed first. `aside` opens the pick in
    /// the group to the right, as ⌘Enter always does.
    pub fn open(
        wb: &mut Workbench,
        kind: GotoKind,
        locations: Vec<ResolvedLocation>,
        current: Arc<Path>,
        aside: bool,
        window: &mut Window,
        cx: &mut Context<Workbench>,
    ) {
        wb.close_locations_picker(window, cx);
        let focus_before = window.focused(cx);
        let documents = wb.documents().downgrade();
        let root: Arc<Path> = Arc::from(wb.root());
        let locations = locations
            .into_iter()
            .map(|mut l| {
                l.path = wb.absolute_path(&l.path);
                l
            })
            .collect();
        let current = wb.absolute_path(&current);
        let picker = cx.new(|cx| {
            let mut picker = LocationsPicker::new(kind, locations, &current, &root, documents, cx);
            picker.aside = aside;
            picker
        });
        let events = cx.subscribe_in(&picker, window, Workbench::on_locations_picker_event);
        window.focus(&picker.focus_handle(cx), cx);
        wb.locations_picker = Some(ShownLocationsPicker {
            picker,
            focus_before,
            _events: events,
        });
        cx.notify();
    }

    fn new(
        kind: GotoKind,
        mut locations: Vec<ResolvedLocation>,
        current: &Path,
        root: &Path,
        documents: WeakEntity<Documents>,
        cx: &mut Context<Self>,
    ) -> Self {
        locations.sort_by(|a, b| {
            let key = |l: &ResolvedLocation| {
                (
                    *l.path != *current,
                    l.path.clone(),
                    l.range.start.row,
                    l.range.start.column,
                )
            };
            key(a).cmp(&key(b))
        });
        let mut files: Vec<FileGroup> = Vec::new();
        let mut location_files = Vec::with_capacity(locations.len());
        for location in &locations {
            if files.last().is_none_or(|f| f.path != location.path) {
                files.push(file_group(&location.path, root));
            }
            location_files.push(files.len() - 1);
        }
        let excerpts: Vec<Excerpt> = locations.iter().map(excerpt).collect();
        let haystacks = excerpts
            .iter()
            .zip(&location_files)
            .map(|(e, &file)| format!("{} {}", files[file].name, e.text))
            .collect();
        let digits = locations
            .iter()
            .map(|l| (l.range.start.row + 1).ilog10() as usize + 1)
            .max()
            .unwrap_or(1);
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder(placeholder(kind));
            input.on_change(move |text, _, cx| {
                this.update(cx, |picker, cx| picker.query_changed(text, cx))
                    .ok();
            });
            input
        });
        let mut picker = LocationsPicker {
            input,
            documents,
            locations,
            location_files,
            excerpts,
            haystacks,
            files,
            rows: Vec::new(),
            selected: None,
            aside: false,
            scroll: UniformListScrollHandle::new(),
            preview: None,
            _preview_loaded: None,
            digits,
        };
        picker.query_changed("", cx);
        picker
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The selected location.
    pub fn selected(&self) -> Option<&ResolvedLocation> {
        match self.rows.get(self.selected?)? {
            Row::Location { index, .. } => self.locations.get(*index),
            Row::Header { .. } => None,
        }
    }

    /// The preview of the selected location.
    pub fn preview(&self) -> Option<&Entity<CodeView>> {
        self.preview.as_ref()
    }

    /// Each row as plain text: `"header:<file name>"` or
    /// `"<line>:<excerpt>"`, the line 1-based and the excerpt the shown
    /// part of the line (leading whitespace dropped, long lines cut).
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::Header { file, .. } => format!("header:{}", self.files[*file].name),
                Row::Location { index, .. } => {
                    let row = self.locations[*index].range.start.row + 1;
                    format!("{row}:{}", self.excerpts[*index].text)
                }
            })
            .collect()
    }

    /// Keeps the locations whose "file name + line text" fuzzy-matches
    /// `text`, in their sorted order under their file headers, and
    /// selects the first.
    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut matched: HashMap<usize, Vec<u32>> =
            fuzzy_filter(self.haystacks.iter().map(String::as_str), text.trim())
                .into_iter()
                .collect();
        let mut rows = Vec::new();
        let mut header: Option<usize> = None;
        for index in 0..self.locations.len() {
            let Some(positions) = matched.remove(&index) else {
                continue;
            };
            let file = self.location_files[index];
            if header.is_none_or(|h| !matches!(rows[h], Row::Header { file: f, .. } if f == file)) {
                header = Some(rows.len());
                rows.push(Row::Header { file, count: 0 });
            }
            if let Some(Row::Header { count, .. }) = header.map(|h| &mut rows[h]) {
                *count += 1;
            }
            // Positions past the file name and its space are in the line.
            let skip = self.files[file].name.chars().count() as u32 + 1;
            let positions = positions
                .into_iter()
                .filter_map(|p| p.checked_sub(skip))
                .collect();
            rows.push(Row::Location { index, positions });
        }
        self.rows = rows;
        let first = self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Location { .. }));
        self.select(first, cx);
    }

    /// Selects row `row` (a location) and previews it.
    fn select(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        self.selected = row;
        if let Some(row) = row {
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
        self.update_preview(cx);
        cx.notify();
    }

    /// The next location row from the selected one by `delta` (±1),
    /// skipping headers and wrapping at both ends.
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.rows.len() as isize;
        let Some(start) = self.selected else {
            return;
        };
        let mut row = start as isize;
        for _ in 0..len {
            row = (row + delta).rem_euclid(len);
            if matches!(self.rows[row as usize], Row::Location { .. }) {
                self.select(Some(row as usize), cx);
                return;
            }
        }
    }

    /// Shows the selected location in the preview: the file through
    /// `Documents` (sharing its text with open tabs), the range centred
    /// and its rows highlighted. A file still loading is centred once its
    /// text arrives.
    fn update_preview(&mut self, cx: &mut Context<Self>) {
        let Some(location) = self.selected().cloned() else {
            self.preview = None;
            self._preview_loaded = None;
            return;
        };
        let Some(documents) = self.documents.upgrade() else {
            return;
        };
        let view = match &self.preview {
            Some(view) if *view.read(cx).path() == location.path => view.clone(),
            _ => {
                let path = location.path.clone();
                let view = documents.update(cx, |docs, cx| docs.open_preview(path, cx));
                self.preview = Some(view.clone());
                view
            }
        };
        let range = &location.range;
        let rows = highlighted_rows(range);
        let start = range.start;
        view.update(cx, |v, cx| v.set_line_highlights(vec![rows], cx));
        // A new view of a loaded document has its text at once.
        if view.read(cx).revision() > 0 {
            self._preview_loaded = None;
            view.update(cx, |v, cx| v.go_to(start, cx));
            return;
        }
        self._preview_loaded = Some(cx.observe(&view, move |picker, view, cx| {
            if view.read(cx).revision() > 0 {
                picker._preview_loaded = None;
                view.update(cx, |v, cx| v.go_to(start, cx));
            }
        }));
    }

    /// Opens the location at `row`; `to_side` in the group to the right.
    fn confirm(&mut self, row: usize, to_side: bool, cx: &mut Context<Self>) {
        let Some(Row::Location { index, .. }) = self.rows.get(row) else {
            return;
        };
        cx.emit(LocationsPickerEvent::Open {
            location: self.locations[*index].clone(),
            aside: to_side || self.aside,
        });
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift {
            return;
        }
        match (ev.keystroke.key.as_str(), m.platform) {
            ("up", false) => self.move_selection(-1, cx),
            ("down", false) => self.move_selection(1, cx),
            ("enter", to_side) => {
                if let Some(row) = self.selected {
                    self.confirm(row, to_side, cx);
                }
            }
            // Unless a key binding closed the picker first.
            ("escape", false) => cx.emit(LocationsPickerEvent::Dismiss),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// A click selects; a double click opens.
    fn clicked(&mut self, row: usize, ev: &ClickEvent, cx: &mut Context<Self>) {
        if ev.click_count() >= 2 {
            self.confirm(row, false, cx);
        } else if self.selected != Some(row) {
            self.select(Some(row), cx);
        }
    }

    fn render_header(&self, row: usize, file: usize, count: usize, cx: &App) -> AnyElement {
        let state = cx.global::<AppState>();
        let ui = &state.theme.ui;
        let group = &self.files[file];
        let icon = state.icons.file(&group.path);
        div()
            .id(("locations-picker-row", row))
            .debug_selector(|| format!("locations-picker-row-{row}"))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2. * GRID))
            .h(px(PICKER_ROW_HEIGHT))
            .px(px(2. * GRID))
            .overflow_hidden()
            .child(file_icon(&icon, color(ui.icon_muted).into()))
            .child(div().flex_none().child(group.name.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(color(ui.text_muted))
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .child(group.dir.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(color(ui.text_muted))
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .child(count.to_string()),
            )
            .into_any_element()
    }

    fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let font_family = editor_font_family(None, cx);
        range
            .filter_map(|row| {
                Some(match self.rows.get(row)? {
                    Row::Header { file, count } => self.render_header(row, *file, *count, cx),
                    Row::Location { index, positions } => {
                        self.render_location(row, *index, positions, &font_family, cx)
                    }
                })
            })
            .collect()
    }

    fn render_location(
        &self,
        row: usize,
        index: usize,
        positions: &[u32],
        font_family: &SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let location = &self.locations[index];
        let number_width = px(self.digits as f32 * UI_FONT_SIZE * CODE_CHAR_EM);
        let excerpt = &self.excerpts[index];
        let text = line_text(
            &excerpt.text,
            excerpt.range.clone(),
            positions,
            color(ui.search_match_background).into(),
            match_color(cx),
        );
        picker_row("locations-picker-row", row, self.selected == Some(row), cx)
            .on_click(cx.listener(move |picker, ev, _, cx| picker.clicked(row, ev, cx)))
            .gap(px(2. * GRID))
            .font_family(font_family.clone())
            .child(
                div()
                    .flex_none()
                    .w(number_width)
                    .flex()
                    .justify_end()
                    .text_color(color(ui.text_muted))
                    .child((location.range.start.row + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(text),
            )
            .into_any_element()
    }
}

/// `"Filter definitions…"` and the like.
fn placeholder(kind: GotoKind) -> &'static str {
    match kind {
        GotoKind::Definition => "Filter definitions…",
        GotoKind::TypeDefinition => "Filter type definitions…",
        GotoKind::Implementation => "Filter implementations…",
        GotoKind::Declaration => "Filter declarations…",
        GotoKind::References => "Filter references…",
    }
}

/// The name of `path` and its directory, relative to `root` when inside.
fn file_group(path: &Arc<Path>, root: &Path) -> FileGroup {
    let shown = path.strip_prefix(root).unwrap_or(path);
    let name = shown
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| shown.to_string_lossy().into_owned());
    let dir = shown
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    FileGroup {
        name: name.into(),
        dir: dir.into(),
        path: path.clone(),
    }
}

/// The rows a range covers, half-open: an end at column 0 of a later row
/// does not count that row.
pub(crate) fn highlighted_rows(range: &Range<lucerna_text::Point>) -> Range<u32> {
    let (start, end) = (range.start, range.end);
    let last = if end.column == 0 && end.row > start.row {
        end.row - 1
    } else {
        end.row.max(start.row)
    };
    start.row..last + 1
}

/// The largest char boundary of `text` at or before `byte`.
fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// What the row of `location` shows: its line without leading
/// whitespace, cut to [`EXCERPT_CHARS`] chars (`…` marking a cut) with at
/// most [`EXCERPT_BEFORE`] before the range. Reads only near the range,
/// however long the line.
fn excerpt(location: &ResolvedLocation) -> Excerpt {
    let line = location.line_text.as_str();
    let text = line.trim_start();
    let trimmed = line.len() - text.len();
    let range = &location.range;
    let start = floor_boundary(text, (range.start.column as usize).saturating_sub(trimmed));
    let end = if range.end.row == range.start.row {
        floor_boundary(text, (range.end.column as usize).saturating_sub(trimmed)).max(start)
    } else {
        text.len()
    };
    if text.char_indices().nth(EXCERPT_CHARS).is_none() {
        return Excerpt {
            text: SharedString::from(text.to_owned()),
            range: start..end,
        };
    }
    let from = text[..start]
        .char_indices()
        .rev()
        .take(EXCERPT_BEFORE)
        .last()
        .map_or(start, |(i, _)| i);
    let prefix = if from > 0 { "…" } else { "" };
    // Room for the prefix and a closing `…`.
    let budget = EXCERPT_CHARS - prefix.chars().count() - 1;
    let to = text[from..]
        .char_indices()
        .nth(budget)
        .map_or(text.len(), |(i, _)| from + i);
    let suffix = if to < text.len() { "…" } else { "" };
    let shown = format!("{prefix}{}{suffix}", &text[from..to]);
    let shift = |b: usize| b.clamp(from, to) - from + prefix.len();
    Excerpt {
        text: shown.into(),
        range: shift(start)..shift(end),
    }
}

/// `line` with `range` (bytes, clamped to char boundaries) on `range_bg`
/// and the chars at `positions` (char indices) in `accent`.
fn line_text(
    line: &SharedString,
    range: Range<usize>,
    positions: &[u32],
    range_bg: gpui::Hsla,
    accent: gpui::Hsla,
) -> StyledText {
    let mut runs: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (i, (byte, ch)) in line.char_indices().enumerate() {
        let style = HighlightStyle {
            color: positions
                .binary_search(&(i as u32))
                .is_ok()
                .then_some(accent),
            background_color: range.contains(&byte).then_some(range_bg),
            ..Default::default()
        };
        if style == HighlightStyle::default() {
            continue;
        }
        let next = byte + ch.len_utf8();
        match runs.last_mut() {
            Some((r, s)) if r.end == byte && *s == style => r.end = next,
            _ => runs.push((byte..next, style)),
        }
    }
    StyledText::new(line.clone()).with_highlights(runs)
}

impl Focusable for LocationsPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for LocationsPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = (viewport.width * WIDTH_SHARE).min(px(MAX_WIDTH));
        let height = (viewport.height * HEIGHT_SHARE).min(px(MAX_HEIGHT));
        let count = self.rows.len();
        let border = color(cx.global::<AppState>().theme.ui.border);
        popover(cx)
            .id("locations-picker")
            .debug_selector(|| "locations-picker".into())
            .occlude()
            .w(width)
            .max_w(width)
            .h(height)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(LocationsPickerEvent::Dismiss)))
            .child(picker_input_row(&self.input, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w(relative(LIST_SHARE))
                            .h_full()
                            .p(px(GRID))
                            .text_size(px(UI_FONT_SIZE))
                            .child(
                                // Only the rows on screen are built: there
                                // may be thousands of references.
                                uniform_list(
                                    "locations-picker-list",
                                    count,
                                    cx.processor(|picker, range, _, cx| {
                                        picker.render_rows(range, cx)
                                    }),
                                )
                                .size_full()
                                .track_scroll(&self.scroll),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "locations-preview".into())
                            .w(relative(1. - LIST_SHARE))
                            .h_full()
                            .border_l_1()
                            .border_color(border)
                            .children(self.preview.clone()),
                    ),
            )
    }
}

impl Workbench {
    /// The shown locations picker, if any.
    pub fn locations_picker(&self) -> Option<&Entity<LocationsPicker>> {
        self.locations_picker.as_ref().map(|shown| &shown.picker)
    }

    /// Hides the locations picker, dropping its preview, and gives focus
    /// back to where it was, unless a click outside already moved it.
    pub(crate) fn close_locations_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.locations_picker.take() else {
            return;
        };
        let focused = window.focused(cx);
        if focused.is_none() || shown.picker.focus_handle(cx).contains_focused(window, cx) {
            match shown.focus_before {
                Some(focus) => window.focus(&focus, cx),
                None => self.focus_editor_area(window, cx),
            }
            // gotoAndPeek jumped while it was open: the view focused before
            // may be hidden now.
            self.refocus_if_hidden(window, cx);
        }
        cx.notify();
    }

    fn on_locations_picker_event(
        &mut self,
        _: &Entity<LocationsPicker>,
        event: &LocationsPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_locations_picker(window, cx);
        if let LocationsPickerEvent::Open { location, aside } = event {
            open_location(self, location, *aside, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        assert_eq!(placeholder(GotoKind::Definition), "Filter definitions…");
        assert_eq!(placeholder(GotoKind::References), "Filter references…");
        assert_eq!(
            placeholder(GotoKind::Implementation),
            "Filter implementations…"
        );
    }

    fn loc(line: &str, start: u32, end: u32) -> ResolvedLocation {
        let p = |column| lucerna_text::Point { row: 0, column };
        ResolvedLocation {
            path: Arc::from(Path::new("/r/a.rs")),
            range: p(start)..p(end),
            line_text: line.to_owned(),
        }
    }

    #[test]
    fn excerpt_short_line_trimmed() {
        let e = excerpt(&loc("    let x = 1;", 8, 9));
        assert_eq!(&*e.text, "let x = 1;");
        assert_eq!(&e.text[e.range], "x");
    }

    #[test]
    fn excerpt_long_line_cut_around_range() {
        // Multi-byte chars so a naive byte cut would split one.
        let line = format!("{}needle{}", "é".repeat(100_000), "ü".repeat(100_000));
        let start = "é".len() as u32 * 100_000;
        let e = excerpt(&loc(&line, start, start + 6));
        assert!(e.text.chars().count() <= EXCERPT_CHARS);
        assert_eq!(&e.text[e.range.clone()], "needle");
        assert!(e.text.starts_with('…') && e.text.ends_with('…'));
        // At the start of a long line: no leading ellipsis.
        let e = excerpt(&loc(&format!("needle{}", "x".repeat(1000)), 0, 6));
        assert_eq!(e.text.chars().count(), EXCERPT_CHARS);
        assert_eq!(&e.text[e.range], "needle");
    }

    #[test]
    fn rows_end_at_column_zero_exclusive() {
        let p = |row, column| lucerna_text::Point { row, column };
        assert_eq!(highlighted_rows(&(p(1, 4)..p(3, 0))), 1..3);
        assert_eq!(highlighted_rows(&(p(1, 4)..p(3, 2))), 1..4);
        assert_eq!(highlighted_rows(&(p(2, 0)..p(2, 0))), 2..3);
    }

    #[test]
    fn file_group_relative_to_root() {
        let group = file_group(&Arc::from(Path::new("/r/src/b.rs")), Path::new("/r"));
        assert_eq!((&*group.name, &*group.dir), ("b.rs", "src"));
        let outside = file_group(&Arc::from(Path::new("/x/y.rs")), Path::new("/r"));
        assert_eq!((&*outside.name, &*outside.dir), ("y.rs", "/x"));
    }
}
