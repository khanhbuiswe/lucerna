//! Quick Open: one floating picker for files (`⌘P`), commands (`⇧⌘P`,
//! `>` prefix) and go to line (`⌃G`, `:` prefix), and the recently
//! opened files that rank first in it.

mod query;
/// Also the picker building blocks (`picker_input_row`, `picker_list`,
/// `picker_row`, `highlighted`, `wrap_index`) for other pickers.
pub(crate) mod view;

pub use query::{QuickOpenQuery, parse_query};
pub use view::{QuickOpen, QuickOpenEvent};

use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext, Context, Focusable, Window};
use lucerna_text::Point;

use crate::code_view::CodeView;
use crate::{AppState, CommandRegistry, Workbench};
use view::{CommandItem, Sources};

/// How many recently opened files are remembered.
pub const RECENT_FILES_LIMIT: usize = 50;

/// Commands that make no sense in the palette.
const HIDDEN_COMMANDS: &[&str] = &["workbench.action.closeQuickOpen"];

/// Every opened path, newest first, without duplicates, at most
/// [`RECENT_FILES_LIMIT`].
#[derive(Debug, Default, Clone)]
pub struct RecentFiles {
    paths: Vec<Arc<Path>>,
}

impl RecentFiles {
    /// Moves `path` to the front.
    pub fn record(&mut self, path: Arc<Path>) {
        self.paths.retain(|p| *p != path);
        self.paths.insert(0, path);
        self.paths.truncate(RECENT_FILES_LIMIT);
    }

    /// Newest first.
    pub fn paths(&self) -> &[Arc<Path>] {
        &self.paths
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.quickOpen",
        "Go to File…",
        |wb, _, window, cx| wb.show_quick_open("", window, cx),
    );
    commands.register(
        "workbench.action.showCommands",
        "Show All Commands",
        |wb, _, window, cx| wb.show_quick_open(">", window, cx),
    );
    commands.register(
        "workbench.action.gotoLine",
        "Go to Line/Column…",
        |wb, _, window, cx| wb.show_quick_open(":", window, cx),
    );
    commands.register(
        "workbench.action.closeQuickOpen",
        "Close Quick Open",
        |wb, _, window, cx| wb.close_quick_open(window, cx),
    );
}

impl Workbench {
    /// The open Quick Open widget, if any.
    pub fn quick_open(&self) -> Option<&gpui::Entity<QuickOpen>> {
        self.quick_open.as_ref().map(|(qo, _)| qo)
    }

    /// Recently opened files, newest first.
    pub fn recent_files(&self) -> &[Arc<Path>] {
        self.recent_files.paths()
    }

    /// Shows Quick Open with `prefix` typed, or replaces the text of the
    /// one already shown, and focuses its input.
    pub fn show_quick_open(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((qo, _)) = &self.quick_open {
            let input = qo.read(cx).input().clone();
            input.update(cx, |input, cx| input.set_text(prefix, window, cx));
            window.focus(&input.focus_handle(cx), cx);
            return;
        }
        self.focus_before_quick_open = window.focused(cx);
        let keymap = &cx.global::<AppState>().keymap;
        let commands = self
            .commands()
            .all()
            .filter(|(id, _)| !HIDDEN_COMMANDS.contains(id))
            .map(|(id, title)| CommandItem {
                id: id.to_string(),
                title: title.to_string(),
                keybinding: keymap.label_for(id),
            })
            .collect();
        let sources = Sources {
            workbench: cx.weak_entity(),
            snapshot: self.worktree().snapshot(),
            recent: self.recent_files.paths().into(),
            commands,
            has_editor: self.active_code_view(cx).is_some(),
        };
        let qo = cx.new(|cx| QuickOpen::new(prefix, sources, window, cx));
        let events = cx.subscribe_in(&qo, window, Self::on_quick_open_event);
        window.focus(&qo.focus_handle(cx), cx);
        self.quick_open = Some((qo, events));
        cx.notify();
    }

    /// Hides Quick Open and gives focus back to where it was, unless a
    /// click outside already moved it elsewhere.
    pub fn close_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((qo, _)) = self.quick_open.take() else {
            return;
        };
        let previous = self.focus_before_quick_open.take();
        let focused = window.focused(cx);
        if focused.is_none() || qo.focus_handle(cx).contains_focused(window, cx) {
            match previous {
                Some(focus) => window.focus(&focus, cx),
                None => self.focus_editor_area(window, cx),
            }
        }
        cx.notify();
    }

    fn on_quick_open_event(
        &mut self,
        _: &gpui::Entity<QuickOpen>,
        event: &QuickOpenEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_quick_open(window, cx);
        match event {
            QuickOpenEvent::Open { path, mode } => {
                self.open_path(path.clone(), *mode, cx);
                self.focus_editor_area(window, cx);
            }
            QuickOpenEvent::RunCommand(id) => {
                self.run_command(id, None, window, cx);
            }
            QuickOpenEvent::GoToLine { line, column } => {
                self.go_to_line(*line, *column, cx);
                self.focus_editor_area(window, cx);
            }
            QuickOpenEvent::Dismiss => {}
        }
    }

    /// Moves the active code view's cursor to a 1-based `line` and
    /// `column` (in characters); out-of-range values clamp.
    fn go_to_line(&mut self, line: u32, column: Option<u32>, cx: &mut Context<Self>) {
        let Some(view) = self.active_code_view(cx) else {
            return;
        };
        let snapshot = view.read(cx).snapshot();
        let row = line
            .saturating_sub(1)
            .min(snapshot.line_count().saturating_sub(1));
        let column = match column {
            Some(column) if column > 1 => {
                let text = snapshot.line(row);
                text.char_indices()
                    .nth(column as usize - 1)
                    .map_or(text.len(), |(byte, _)| byte) as u32
            }
            _ => 0,
        };
        let point = Point { row, column };
        self.documents()
            .clone()
            .update(cx, |docs, cx| docs.go_to(&view, point, cx));
    }

    /// The code view of the focused group's active tab, once drawn.
    fn active_code_view(&self, cx: &gpui::App) -> Option<gpui::Entity<CodeView>> {
        self.editor_groups()
            .read(cx)
            .active_body()
            .and_then(|body| body.downcast::<CodeView>().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Arc<Path> {
        Arc::from(Path::new(s))
    }

    #[test]
    fn recent_files_newest_first_deduplicated_capped() {
        let mut recent = RecentFiles::default();
        recent.record(p("a"));
        recent.record(p("b"));
        recent.record(p("a"));
        assert_eq!(recent.paths(), &[p("a"), p("b")]);
        for i in 0..60 {
            recent.record(p(&format!("f{i}")));
        }
        assert_eq!(recent.paths().len(), RECENT_FILES_LIMIT);
        assert_eq!(recent.paths()[0], p("f59"));
    }
}
