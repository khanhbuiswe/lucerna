//! The Quick Open popover: a [`TextInput`] over a list of files, commands
//! or a go-to-line row, drawn with the shared [`super::picker`] pieces.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString, Task, WeakEntity, Window, div,
    prelude::*, px,
};
use lucerna_project::{FileMatch, QUICK_OPEN_LIMIT, WorktreeSnapshot, match_files};
use lucerna_text::Point;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, picker_row_sized,
    wrap_index,
};
use super::query::{QuickOpenQuery, parse_query};
use crate::layout::color;
use crate::symbols::SymbolState;
use crate::symbols::quick_open::{NO_SYMBOL_SOURCE, NO_SYMBOLS, SymbolId, SymbolPicker, SymbolRow};
use crate::symbols::workspace::{
    self, NO_SERVER, TYPE_TO_SEARCH, WORKSPACE_ROW_HEIGHT, WORKSPACE_SYMBOLS_DEBOUNCE,
    WorkspaceSymbolItem,
};
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{GRID, popover};
use crate::{AppState, OpenMode, Workbench};

/// A command as the palette lists it.
#[derive(Debug, Clone)]
pub(crate) struct CommandItem {
    pub id: String,
    pub title: String,
    /// The keybinding label, e.g. `⌘B`, if the command is bound.
    pub keybinding: Option<String>,
}

/// What Quick Open reads from the workbench when it opens.
pub(crate) struct Sources {
    pub workbench: WeakEntity<Workbench>,
    pub snapshot: WorktreeSnapshot,
    /// Recently opened files, newest first.
    pub recent: Arc<[Arc<Path>]>,
    pub commands: Vec<CommandItem>,
    /// Whether a code view is active, so go-to-line has a target.
    pub has_editor: bool,
    /// Lists these extension ids instead (the install command): Enter
    /// installs the selected one. `Err` holds the message shown while
    /// there are none yet, such as "Loading extension list…".
    pub extension_ids: Option<ExtensionIds>,
}

/// The ids the install command lists, or the message shown instead.
pub(crate) type ExtensionIds = Result<Vec<String>, &'static str>;

/// What the workbench does when a row is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickOpenEvent {
    Open {
        path: Arc<Path>,
        mode: OpenMode,
    },
    RunCommand(String),
    /// Install the extension with this registry id.
    InstallExtension(String),
    /// 1-based line and column.
    GoToLine {
        line: u32,
        column: Option<u32>,
    },
    /// Put the cursor of the active editor, showing `path`, at `point`
    /// (a symbol picked with `@`).
    GoToSymbol {
        path: Arc<Path>,
        point: Point,
    },
    /// Open a symbol picked with `#`; `to_side` in the group to the right.
    OpenWorkspaceSymbol {
        item: Box<WorkspaceSymbolItem>,
        to_side: bool,
    },
    Dismiss,
}

enum Row {
    File(FileMatch),
    Command {
        index: usize,
        /// Char indices into the title.
        positions: Vec<u32>,
    },
    GoToLine {
        line: u32,
        column: Option<u32>,
    },
    Extension {
        index: usize,
        /// Char indices into the id.
        positions: Vec<u32>,
    },
    /// A `@` row: a document symbol or a group header.
    Symbol(SymbolRow),
    /// A `#` row: a workspace symbol.
    WorkspaceSymbol {
        index: usize,
        /// Char indices into the name.
        positions: Vec<u32>,
        /// Its file and line, as shown.
        location: SharedString,
    },
    Message(&'static str),
}

impl Row {
    /// Group headers cannot be selected.
    fn is_selectable(&self) -> bool {
        match self {
            Row::Symbol(row) => row.is_selectable(),
            _ => true,
        }
    }
}

pub struct QuickOpen {
    workbench: WeakEntity<Workbench>,
    input: Entity<TextInput>,
    snapshot: WorktreeSnapshot,
    recent: Arc<[Arc<Path>]>,
    commands: Rc<[CommandItem]>,
    has_editor: bool,
    extension_ids: Option<Result<Rc<[String]>, &'static str>>,
    /// The active editor's symbols, for `@`; read from the workbench when
    /// `@` is first typed.
    symbols: Option<SymbolPicker>,
    symbols_loaded: bool,
    /// The symbols `#` lists.
    workspace_symbols: Vec<WorkspaceSymbolItem>,
    /// The debounced `#` search of the latest query; replacing it cancels
    /// the requests of the one before.
    _workspace_search: Task<()>,
    rows: Vec<Row>,
    selected: usize,
    scroll: ScrollHandle,
    /// Renders left that scroll to the selection again: gpui resolves
    /// `scroll_to_item` in the list's first layout, before the list knows
    /// its height, so new rows ask once more after that layout.
    rescroll: u8,
    /// Bumped on every query; a background match applies its results
    /// only while its generation is still the latest.
    generation: u64,
    /// Cancels the running file match.
    cancel: Arc<AtomicBool>,
    _matching: Task<()>,
}

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl QuickOpen {
    /// Opens with `prefix` typed in the input: `""` for files, `">"` for
    /// commands, `":"` for go to line.
    pub(crate) fn new(
        prefix: &str,
        sources: Sources,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder(if sources.extension_ids.is_some() {
                "Type an extension id to install"
            } else {
                "Search files by name (> for commands, : to go to a line)"
            });
            input.on_change(move |text, _, cx| {
                this.update(cx, |qo, cx| qo.query_changed(text, cx)).ok();
            });
            input.set_text(prefix, window, cx);
            input
        });
        QuickOpen {
            workbench: sources.workbench,
            input,
            snapshot: sources.snapshot,
            recent: sources.recent,
            commands: sources.commands.into(),
            has_editor: sources.has_editor,
            extension_ids: sources.extension_ids.map(|ids| ids.map(Into::into)),
            symbols: None,
            symbols_loaded: false,
            workspace_symbols: Vec::new(),
            _workspace_search: Task::ready(()),
            rows: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            rescroll: 0,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            _matching: Task::ready(()),
        }
    }

    /// Whether it lists extension ids (the install command).
    pub(crate) fn lists_extensions(&self) -> bool {
        self.extension_ids.is_some()
    }

    /// Whether it lists extension ids that have not arrived yet.
    pub(crate) fn awaits_extension_ids(&self) -> bool {
        matches!(self.extension_ids, Some(Err(_)))
    }

    /// Lists `ids` (or shows the message) in place of what it showed,
    /// keeping the typed text.
    pub(crate) fn set_extension_ids(&mut self, ids: ExtensionIds, cx: &mut Context<Self>) {
        self.extension_ids = Some(ids.map(Into::into));
        let text = self.input.read(cx).text().to_string();
        self.query_changed(&text, cx);
    }

    fn extension_id(&self, index: usize) -> &str {
        match &self.extension_ids {
            Some(Ok(ids)) => &ids[index],
            _ => "",
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The document whose symbols `@` lists.
    pub(crate) fn symbols_path(&self) -> Option<&Arc<Path>> {
        self.symbols.as_ref().map(SymbolPicker::path)
    }

    /// Lists `state` in place of the symbols shown, keeping the typed
    /// text. The selected symbol stays selected (by name and selection
    /// range) when the new symbols still have it, without moving the
    /// editor again; the symbol at the cursor is selected only when
    /// symbols first arrive.
    pub fn set_symbol_state(&mut self, state: SymbolState, cx: &mut Context<Self>) {
        let Some(symbols) = &mut self.symbols else {
            return;
        };
        let keep = match self.rows.get(self.selected) {
            Some(Row::Symbol(SymbolRow::Symbol { index, .. })) => symbols.symbol_id(*index),
            _ => None,
        };
        symbols.set_state(state);
        let text = self.input.read(cx).text().to_string();
        if let QuickOpenQuery::Symbols {
            text,
            group_by_kind,
        } = parse_query(&text)
        {
            self.show_symbols(&text, group_by_kind, keep, cx);
        }
    }

    /// Lists the symbols matching `text`. `keep` is selected again when
    /// listed; otherwise the picker's first choice is, and previewed
    /// unless the query is empty and nothing was previewed yet. A message
    /// instead of rows ends the preview.
    fn show_symbols(
        &mut self,
        text: &str,
        group_by_kind: bool,
        keep: Option<SymbolId>,
        cx: &mut Context<Self>,
    ) {
        let Some(symbols) = &self.symbols else {
            self.set_rows(vec![Row::Message(NO_SYMBOL_SOURCE)], cx);
            return;
        };
        match symbols.rows(text, group_by_kind) {
            Ok((rows, selected)) => {
                let kept = keep.and_then(|id| symbols.index_of(&id)).and_then(|at| {
                    rows.iter()
                        .position(|r| matches!(r, SymbolRow::Symbol { index, .. } if *index == at))
                });
                let previewing = symbols.is_previewing();
                let rows = rows.into_iter().map(Row::Symbol).collect();
                self.set_rows_at(rows, kept.unwrap_or(selected), cx);
                if kept.is_none() && (!text.is_empty() || previewing) {
                    self.preview_selected(cx);
                }
            }
            Err(message) => {
                self.end_preview(cx);
                self.set_rows(vec![Row::Message(message)], cx);
            }
        }
    }

    /// Reads the active editor's symbols from the workbench the first
    /// time `@` is typed. Runs from the input's deferred change callback,
    /// never while the workbench is being updated.
    fn load_symbols(&mut self, cx: &mut Context<Self>) {
        if self.symbols_loaded {
            return;
        }
        self.symbols_loaded = true;
        self.symbols = self
            .workbench
            .upgrade()
            .and_then(|wb| wb.update(cx, |wb, cx| wb.symbols_context(cx)))
            .map(SymbolPicker::new);
    }

    /// Puts the editor a `@` preview moved back where it was.
    pub(crate) fn end_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(symbols) = &mut self.symbols {
            symbols.end_preview(cx);
        }
    }

    /// The text in the input, prefix included.
    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The list's vertical scroll offset: negative once scrolled down.
    pub fn list_scroll_offset(&self) -> gpui::Pixels {
        self.scroll.offset().y
    }

    /// Each row as plain text, the parts joined by spaces: `"main.rs src"`,
    /// `"View: Toggle Word Wrap ⌥Z"`, `"Go to line 12"`.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| {
                if let (Row::Symbol(row), Some(symbols)) = (row, &self.symbols) {
                    return symbols.row_text(row);
                }
                if let Row::WorkspaceSymbol {
                    index, location, ..
                } = row
                {
                    return workspace::row_text(&self.workspace_symbols[*index], location);
                }
                let (main, detail) = self.row_parts(row);
                match detail {
                    Some(detail) if !detail.is_empty() => format!("{main} {detail}"),
                    _ => main.to_string(),
                }
            })
            .collect()
    }

    /// The main text and the dimmed detail (directory or keybinding).
    fn row_parts(&self, row: &Row) -> (SharedString, Option<SharedString>) {
        match row {
            Row::File(m) => {
                let (name, dir) = split_path(&m.path);
                (name.into(), Some(dir.into()))
            }
            Row::Command { index, .. } => {
                let c = &self.commands[*index];
                (c.title.clone().into(), c.keybinding.clone().map(Into::into))
            }
            Row::GoToLine { line, column } => (go_to_line_label(*line, *column).into(), None),
            Row::Extension { index, .. } => (self.extension_id(*index).to_string().into(), None),
            Row::Symbol(_) | Row::WorkspaceSymbol { .. } => (SharedString::default(), None),
            Row::Message(text) => ((*text).into(), None),
        }
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.generation += 1;
        self.cancel.store(true, Ordering::Relaxed);
        self._workspace_search = Task::ready(());
        match &self.extension_ids {
            Some(Ok(ids)) => {
                // Hundreds of ids: only the best ones are drawn.
                let rows = fuzzy_filter(ids.iter().map(String::as_str), text.trim())
                    .into_iter()
                    .take(QUICK_OPEN_LIMIT)
                    .map(|(index, positions)| Row::Extension { index, positions })
                    .collect();
                self.set_rows(rows, cx);
                return;
            }
            Some(Err(message)) => {
                self.set_rows(vec![Row::Message(message)], cx);
                return;
            }
            None => {}
        }
        let query = parse_query(text);
        if !matches!(query, QuickOpenQuery::Symbols { .. }) {
            self.end_preview(cx);
        }
        match query {
            QuickOpenQuery::Files(query) => self.match_files(query, cx),
            QuickOpenQuery::Commands(query) => {
                let rows = filter_commands(&self.commands, &query);
                self.set_rows(rows, cx);
            }
            QuickOpenQuery::GotoLine { line, column } => {
                let row = match line {
                    _ if !self.has_editor => Row::Message("Open a file to go to a line"),
                    Some(line) => Row::GoToLine { line, column },
                    None => Row::Message("Type a line number…"),
                };
                self.set_rows(vec![row], cx);
            }
            QuickOpenQuery::Symbols {
                text,
                group_by_kind,
            } => {
                self.load_symbols(cx);
                self.show_symbols(&text, group_by_kind, None, cx);
            }
            QuickOpenQuery::WorkspaceSymbols(text) => self.search_workspace_symbols(text, cx),
        }
    }

    /// Asks the language servers for the symbols matching `text` once
    /// typing pauses for 100 ms. The symbols shown stay until the answer
    /// arrives; other rows are cleared.
    fn search_workspace_symbols(&mut self, text: String, cx: &mut Context<Self>) {
        if text.is_empty() {
            self.set_rows(vec![Row::Message(TYPE_TO_SEARCH)], cx);
            return;
        }
        let Some(wb) = self.workbench.upgrade() else {
            return;
        };
        let servers = wb.read(cx).language_servers().clone();
        if !servers.read(cx).has_workspace_symbols() {
            self.set_rows(vec![Row::Message(NO_SERVER)], cx);
            return;
        }
        if !matches!(self.rows.first(), Some(Row::WorkspaceSymbol { .. })) {
            self.set_rows(Vec::new(), cx);
        }
        let generation = self.generation;
        self._workspace_search = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(WORKSPACE_SYMBOLS_DEBOUNCE)
                .await;
            let search = this.update(cx, |qo, cx| {
                let wb = qo.workbench.upgrade()?;
                let project = wb.read(cx).worktree().snapshot();
                Some(servers.update(cx, |s, cx| s.workspace_symbols(text.clone(), project, cx)))
            });
            let Ok(Some(search)) = search else {
                return;
            };
            let items = search.await;
            this.update(cx, |qo, cx| {
                if qo.generation == generation {
                    qo.show_workspace_symbols(items, &text, cx);
                }
            })
            .ok();
        });
    }

    /// Lists `items`, the answer for `text`, or "No symbols found".
    fn show_workspace_symbols(
        &mut self,
        items: Vec<WorkspaceSymbolItem>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            self.workspace_symbols.clear();
            self.set_rows(vec![Row::Message(NO_SYMBOLS)], cx);
            return;
        }
        let Some(wb) = self.workbench.upgrade() else {
            return;
        };
        let root = wb.read(cx).root();
        let rows = items
            .iter()
            .enumerate()
            .map(|(index, item)| Row::WorkspaceSymbol {
                index,
                positions: workspace::match_positions(&item.name, text),
                location: workspace::location_label(item, root).into(),
            })
            .collect();
        self.workspace_symbols = items;
        self.set_rows(rows, cx);
    }

    /// Matches files on the background executor; the old rows stay until
    /// the results of the latest query arrive.
    fn match_files(&mut self, query: String, cx: &mut Context<Self>) {
        if let Some(wb) = self.workbench.upgrade() {
            self.snapshot = wb.read(cx).worktree().snapshot();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (snapshot, recent) = (self.snapshot.clone(), self.recent.clone());
        let generation = self.generation;
        let matching = cx.background_spawn(async move {
            guarded("quick-open", || {
                match_files(&snapshot, &query, &recent, QUICK_OPEN_LIMIT, &cancel)
            })
            .unwrap_or_default()
        });
        self._matching = cx.spawn(async move |this, cx| {
            let matches = matching.await;
            this.update(cx, |qo, cx| {
                if qo.generation == generation {
                    qo.set_rows(matches.into_iter().map(Row::File).collect(), cx);
                }
            })
            .ok();
        });
    }

    fn set_rows(&mut self, rows: Vec<Row>, cx: &mut Context<Self>) {
        self.set_rows_at(rows, 0, cx);
    }

    fn set_rows_at(&mut self, rows: Vec<Row>, selected: usize, cx: &mut Context<Self>) {
        self.rows = rows;
        self.selected = selected;
        self.scroll.scroll_to_item(selected);
        self.rescroll = 2;
        cx.notify();
    }

    /// Moves by `delta` rows, wrapping and skipping group headers.
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.rows.len();
        let mut selected = wrap_index(self.selected, len, delta);
        for _ in 0..len {
            if self.rows.get(selected).is_none_or(Row::is_selectable) {
                break;
            }
            selected = wrap_index(selected, len, delta.signum());
        }
        self.selected = selected;
        self.scroll.scroll_to_item(self.selected);
        self.preview_selected(cx);
        cx.notify();
    }

    /// Shows the selected `@` symbol in the editor.
    fn preview_selected(&mut self, cx: &mut Context<Self>) {
        if let (Some(Row::Symbol(SymbolRow::Symbol { index, .. })), Some(symbols)) =
            (self.rows.get(self.selected), &mut self.symbols)
        {
            symbols.preview(*index, cx);
        }
    }

    /// Activates row `index`; `to_side` opens a file in a new group.
    fn confirm(&mut self, index: usize, to_side: bool, cx: &mut Context<Self>) {
        let event = match self.rows.get(index) {
            Some(Row::File(m)) => QuickOpenEvent::Open {
                path: m.path.clone(),
                mode: if to_side {
                    OpenMode::ToSide
                } else {
                    OpenMode::Preview
                },
            },
            Some(Row::Command { index, .. }) => {
                QuickOpenEvent::RunCommand(self.commands[*index].id.clone())
            }
            Some(&Row::GoToLine { line, column }) => QuickOpenEvent::GoToLine { line, column },
            Some(Row::Extension { index, .. }) => {
                QuickOpenEvent::InstallExtension(self.extension_id(*index).to_string())
            }
            Some(Row::Symbol(SymbolRow::Symbol { index, .. })) => {
                let Some(symbols) = &self.symbols else {
                    return;
                };
                let Some(point) = symbols.target(*index) else {
                    return;
                };
                QuickOpenEvent::GoToSymbol {
                    path: symbols.path().clone(),
                    point,
                }
            }
            Some(Row::WorkspaceSymbol { index, .. }) => QuickOpenEvent::OpenWorkspaceSymbol {
                item: Box::new(self.workspace_symbols[*index].clone()),
                to_side,
            },
            Some(Row::Symbol(SymbolRow::Header { .. }) | Row::Message(_)) | None => return,
        };
        cx.emit(event);
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift {
            return;
        }
        match (ev.keystroke.key.as_str(), m.platform) {
            ("up", false) => self.move_selection(-1, cx),
            ("down", false) => self.move_selection(1, cx),
            ("enter", to_side) => self.confirm(self.selected, to_side, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = &self.rows[index];
        if let (Row::Symbol(symbol_row), Some(symbols)) = (row, &self.symbols) {
            let content = symbols.render_row(symbol_row, cx);
            if !symbol_row.is_selectable() {
                return content;
            }
            return picker_row("quick-open-row", index, index == self.selected, cx)
                .on_click(cx.listener(move |qo, _, _, cx| qo.confirm(index, false, cx)))
                .child(content)
                .into_any_element();
        }
        if let Row::WorkspaceSymbol {
            index: item,
            positions,
            location,
        } = row
        {
            let content = workspace::render_row(
                &self.workspace_symbols[*item],
                positions,
                location.clone(),
                cx,
            );
            return picker_row_sized(
                "quick-open-row",
                index,
                index == self.selected,
                WORKSPACE_ROW_HEIGHT,
                cx,
            )
            .on_click(cx.listener(move |qo, _, _, cx| qo.confirm(index, false, cx)))
            .child(content)
            .into_any_element();
        }
        let ui = &cx.global::<AppState>().theme.ui;
        let (main, detail) = self.row_parts(row);
        let positions: &[u32] = match row {
            Row::File(m) => &m.positions,
            Row::Command { positions, .. } | Row::Extension { positions, .. } => positions,
            _ => &[],
        };
        let content = match row {
            Row::File(m) => {
                // Positions index the whole relative path; the name is its tail.
                let name_start =
                    (m.path.to_string_lossy().chars().count() - main.chars().count()) as u32;
                let name_positions: Vec<u32> = positions
                    .iter()
                    .filter_map(|&p| p.checked_sub(name_start))
                    .collect();
                let dir_positions: Vec<u32> = positions
                    .iter()
                    .copied()
                    .filter(|&p| p < name_start)
                    .collect();
                div()
                    .flex()
                    .gap(px(2. * GRID))
                    .min_w_0()
                    .child(highlighted(main, &name_positions, cx))
                    .child(
                        div()
                            .text_color(color(ui.text_muted))
                            .truncate()
                            .child(highlighted(detail.unwrap_or_default(), &dir_positions, cx)),
                    )
            }
            Row::Command { .. } => div()
                .flex()
                .flex_1()
                .justify_between()
                .gap(px(2. * GRID))
                .child(highlighted(main, positions, cx))
                .children(detail.map(|label| div().text_color(color(ui.text_muted)).child(label))),
            Row::Extension { .. } => div().child(highlighted(main, positions, cx)),
            _ => div().child(main),
        };
        picker_row("quick-open-row", index, index == self.selected, cx)
            .on_click(cx.listener(move |qo, _, _, cx| qo.confirm(index, false, cx)))
            .child(content)
            .into_any_element()
    }
}

impl Focusable for QuickOpen {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for QuickOpen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.rescroll > 0 {
            self.rescroll -= 1;
            self.scroll.scroll_to_item(self.selected);
            if self.rescroll > 0 {
                cx.notify();
            }
        }
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|i| self.render_row(i, cx))
            .collect();
        popover(cx)
            .id("quick-open")
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(QuickOpenEvent::Dismiss)))
            .child(picker_input_row(&self.input, cx))
            .when(!rows.is_empty(), |p| {
                p.child(picker_list("quick-open-list", &self.scroll).children(rows))
            })
    }
}

/// `"main.rs"` and `"src"` for `src/main.rs`; the directory is empty at
/// the root.
fn split_path(path: &Path) -> (String, String) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let dir = path
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    (name, dir)
}

fn go_to_line_label(line: u32, column: Option<u32>) -> String {
    match column {
        Some(column) => format!("Go to line {line}, column {column}"),
        None => format!("Go to line {line}"),
    }
}

/// Commands whose title or id fuzzy-matches `query`, best first; all of
/// them by title when `query` is empty. Highlights come from the title.
fn filter_commands(commands: &[CommandItem], query: &str) -> Vec<Row> {
    if query.is_empty() {
        let mut order: Vec<usize> = (0..commands.len()).collect();
        order.sort_by(|&a, &b| commands[a].title.cmp(&commands[b].title));
        return order
            .into_iter()
            .map(|index| Row::Command {
                index,
                positions: Vec::new(),
            })
            .collect();
    }
    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize, Vec<u32>)> = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        let mut positions = Vec::new();
        let title = pattern.indices(
            Utf32Str::new(&command.title, &mut buf),
            &mut matcher,
            &mut positions,
        );
        let id = pattern.score(Utf32Str::new(&command.id, &mut buf), &mut matcher);
        let Some(score) = title.max(id) else {
            continue;
        };
        if title.is_none() {
            positions.clear();
        }
        positions.sort_unstable();
        positions.dedup();
        scored.push((score, index, positions));
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| commands[a.1].title.cmp(&commands[b.1].title))
    });
    scored
        .into_iter()
        .map(|(_, index, positions)| Row::Command { index, positions })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_root_and_nested() {
        assert_eq!(
            split_path(Path::new("src/main.rs")),
            ("main.rs".into(), "src".into())
        );
        assert_eq!(
            split_path(Path::new("README.md")),
            ("README.md".into(), String::new())
        );
    }

    #[test]
    fn commands_filter_by_title_and_id() {
        let commands = vec![
            CommandItem {
                id: "editor.action.toggleWordWrap".into(),
                title: "View: Toggle Word Wrap".into(),
                keybinding: None,
            },
            CommandItem {
                id: "workbench.view.explorer".into(),
                title: "View: Show Explorer".into(),
                keybinding: None,
            },
        ];
        let first = |q: &str| match filter_commands(&commands, q).first() {
            Some(Row::Command { index, .. }) => Some(*index),
            _ => None,
        };
        assert_eq!(first("word wrap"), Some(0));
        assert_eq!(first("explorer"), Some(1));
        assert_eq!(first("workbench.view"), Some(1), "matches the id");
        assert_eq!(first("zzz"), None);
        assert_eq!(filter_commands(&commands, "").len(), 2);
    }
}
