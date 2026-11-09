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
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::picker::{highlighted, picker_input_row, picker_list, picker_row, wrap_index};
use super::query::{QuickOpenQuery, parse_query};
use crate::layout::color;
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
}

/// What the workbench does when a row is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickOpenEvent {
    Open {
        path: Arc<Path>,
        mode: OpenMode,
    },
    RunCommand(String),
    /// 1-based line and column.
    GoToLine {
        line: u32,
        column: Option<u32>,
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
    Message(&'static str),
}

pub struct QuickOpen {
    workbench: WeakEntity<Workbench>,
    input: Entity<TextInput>,
    snapshot: WorktreeSnapshot,
    recent: Arc<[Arc<Path>]>,
    commands: Rc<[CommandItem]>,
    has_editor: bool,
    rows: Vec<Row>,
    selected: usize,
    scroll: ScrollHandle,
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
            input.set_placeholder("Search files by name (> for commands, : to go to a line)");
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
            rows: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            _matching: Task::ready(()),
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The text in the input, prefix included.
    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Each row as plain text, the parts joined by spaces: `"main.rs src"`,
    /// `"View: Toggle Word Wrap ⌥Z"`, `"Go to line 12"`.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| {
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
            Row::Message(text) => ((*text).into(), None),
        }
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.generation += 1;
        self.cancel.store(true, Ordering::Relaxed);
        match parse_query(text) {
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
        }
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
        self.rows = rows;
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.selected = wrap_index(self.selected, self.rows.len(), delta);
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
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
            Some(Row::Message(_)) | None => return,
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
        let ui = &cx.global::<AppState>().theme.ui;
        let (main, detail) = self.row_parts(row);
        let positions: &[u32] = match row {
            Row::File(m) => &m.positions,
            Row::Command { positions, .. } => positions,
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
