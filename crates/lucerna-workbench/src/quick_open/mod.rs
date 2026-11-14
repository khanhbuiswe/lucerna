//! Quick Open: one floating picker for files (`⌘P`), commands (`⇧⌘P`,
//! `>` prefix), go to line (`⌃G`, `:` prefix), the editor's symbols
//! (`⇧⌘O`, `@`) and the workspace's (`⌘T`, `#`), and the recently opened
//! files that rank first in it.

/// The picker building blocks shared with the theme picker.
pub(crate) mod picker;
mod query;
mod view;

pub use query::{QuickOpenQuery, parse_query};
pub use view::{QuickOpen, QuickOpenEvent};

use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext, Context, Focusable, Window};
use lucerna_text::Point;

use crate::code_view::CodeView;
use crate::extensions::ExtensionService;
use crate::goto::open_at;
use crate::nav_history::NavEntry;
use crate::{AppState, CommandRegistry, OpenMode, Workbench};
use view::{CommandItem, ExtensionIds, Sources};

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

    /// After `from` was renamed to `to`: paths in or under `from` become
    /// the moved paths, keeping their place.
    pub(crate) fn retarget(&mut self, from: &Path, to: &Path) {
        for path in &mut self.paths {
            if let Some(moved) = crate::workspace_state::moved_path(path, from, to) {
                *path = Arc::from(moved);
            }
        }
        let mut seen = std::collections::HashSet::new();
        self.paths.retain(|p| seen.insert(p.clone()));
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
        |wb, _, window, cx| {
            wb.close_quick_open(window, cx);
            wb.close_theme_picker(true, window, cx);
            wb.close_locations_picker(window, cx);
            wb.close_recent_picker(window, cx);
            wb.close_branch_picker(window, cx);
        },
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

    /// Recent files as Quick Open matches them: relative to the root, like
    /// the worktree snapshot. Recent files are absolute (see
    /// [`Workbench::open_path`]); those outside the root are left out.
    fn recent_relative_to_root(&self) -> Arc<[Arc<Path>]> {
        self.recent_files
            .paths()
            .iter()
            .filter_map(|p| p.strip_prefix(self.root()).ok())
            .map(Arc::from)
            .collect()
    }

    /// Shows Quick Open with `prefix` typed, or replaces the text of the
    /// one already shown, and focuses its input.
    pub fn show_quick_open(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((qo, _)) = &self.quick_open {
            if !qo.read(cx).lists_extensions() {
                let input = qo.read(cx).input().clone();
                input.update(cx, |input, cx| input.set_text(prefix, window, cx));
                window.focus(&input.focus_handle(cx), cx);
                return;
            }
            self.close_quick_open(window, cx);
        }
        let keymap = &cx.global::<AppState>().keymap;
        let ctx = self.context_keys(window, cx);
        let commands = self
            .commands()
            .listed(&ctx)
            // Commands without a title are for keys only.
            .filter(|(id, title)| !title.is_empty() && !HIDDEN_COMMANDS.contains(id))
            .map(|(id, title)| CommandItem {
                id: id.to_string(),
                title: title.to_string(),
                keybinding: keymap.label_for(id),
            })
            .collect();
        let sources = Sources {
            workbench: cx.weak_entity(),
            snapshot: self.worktree().snapshot(),
            recent: self.recent_relative_to_root(),
            commands,
            has_editor: self.active_code_view(cx).is_some(),
            extension_ids: None,
        };
        self.open_quick_open(prefix, sources, window, cx);
    }

    /// Shows Quick Open listing `ids` (the install command), or the message
    /// in `Err` until [`Workbench::update_extension_ids`] brings them; Enter
    /// installs the selected one. Replaces a Quick Open already shown.
    pub(crate) fn show_extension_ids(
        &mut self,
        ids: ExtensionIds,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.quick_open.is_some() {
            self.close_quick_open(window, cx);
        }
        let sources = Sources {
            workbench: cx.weak_entity(),
            snapshot: self.worktree().snapshot(),
            recent: Arc::from([]),
            commands: Vec::new(),
            has_editor: false,
            extension_ids: Some(ids),
        };
        self.open_quick_open("", sources, window, cx);
    }

    /// Fills a shown install Quick Open that waits for the extension list
    /// with `ids()`.
    pub(crate) fn update_extension_ids(
        &mut self,
        ids: impl FnOnce(&gpui::App) -> ExtensionIds,
        cx: &mut Context<Self>,
    ) {
        let Some((qo, _)) = &self.quick_open else {
            return;
        };
        if !qo.read(cx).awaits_extension_ids() {
            return;
        }
        let (qo, ids) = (qo.clone(), ids(cx));
        qo.update(cx, |qo, cx| qo.set_extension_ids(ids, cx));
    }

    fn open_quick_open(
        &mut self,
        prefix: &str,
        sources: Sources,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Over another overlay it would hide it, and one Esc would close
        // both.
        self.close_modal_overlays(window, cx);
        self.focus_before_quick_open = window.focused(cx);
        let qo = cx.new(|cx| QuickOpen::new(prefix, sources, window, cx));
        let events = cx.subscribe_in(&qo, window, Self::on_quick_open_event);
        window.focus(&qo.focus_handle(cx), cx);
        self.quick_open = Some((qo, events));
        cx.notify();
    }

    /// Hides Quick Open and gives focus back to where it was, unless a
    /// click outside already moved it elsewhere. An editor a `@` preview
    /// moved goes back where it was.
    pub fn close_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((qo, _)) = self.quick_open.take() else {
            return;
        };
        qo.update(cx, |qo, cx| qo.end_preview(cx));
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
                let path = self.absolute_path(path);
                // Opening the file already shown is no jump; the cursor
                // stays where it is.
                if self.nav_here(cx).is_some_and(|here| here.path != path) {
                    self.record_nav(
                        &NavEntry {
                            path: path.clone(),
                            point: Point { row: 0, column: 0 },
                            scroll_top_row: 0,
                        },
                        cx,
                    );
                }
                self.open_path(path, *mode, cx);
                self.focus_editor_area(window, cx);
            }
            QuickOpenEvent::RunCommand(id) => {
                self.run_command(id, None, window, cx);
            }
            QuickOpenEvent::InstallExtension(id) => {
                if cx.has_global::<ExtensionService>() {
                    ExtensionService::install(id.clone(), cx);
                }
            }
            QuickOpenEvent::GoToLine { line, column } => {
                self.go_to_line(*line, *column, cx);
                self.focus_editor_area(window, cx);
            }
            QuickOpenEvent::GoToSymbol { path, point } => {
                // Closing put the editor back, so the history records
                // where it was before the previews.
                self.jump_to(path.clone(), *point, OpenMode::Preview, cx);
                self.focus_editor_area(window, cx);
            }
            QuickOpenEvent::OpenWorkspaceSymbol { item, to_side } => {
                let (path, aside) = (item.path.clone(), *to_side);
                if let Some(range) = &item.range {
                    open_at(self, path, range.start, aside, window, cx);
                    self.focus_editor_area(window, cx);
                    return;
                }
                let servers = self.language_servers().clone();
                let resolve = servers.update(cx, |s, cx| s.resolve_workspace_symbol(item, cx));
                cx.spawn_in(window, async move |wb, cx| {
                    let point = resolve.await.unwrap_or(Point { row: 0, column: 0 });
                    wb.update_in(cx, |wb, window, cx| {
                        open_at(wb, path, point, aside, window, cx);
                        wb.focus_editor_area(window, cx);
                    })
                    .ok();
                })
                .detach();
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
        self.record_nav(
            &NavEntry {
                path: view.read(cx).path().clone(),
                point,
                scroll_top_row: 0,
            },
            cx,
        );
        self.documents()
            .clone()
            .update(cx, |docs, cx| docs.go_to(&view, point, cx));
    }

    /// The code view of the focused group's active tab, once drawn.
    pub(crate) fn active_code_view(&self, cx: &gpui::App) -> Option<gpui::Entity<CodeView>> {
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
