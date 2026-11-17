//! The workbench side of Vim's `:` command line (spec §8): Quick Open
//! hands the typed line to [`crate::vim::VimLayer`], and the commands it
//! cannot carry out itself (save, close) run here.

use std::path::Path;
use std::sync::Arc;

use gpui::{Context, Entity, Window};
use lucerna_vim::ExCommand;

use crate::Workbench;
use crate::documents::Documents;
use crate::editor_groups::{EditorGroupsView, TabId};
use crate::save::{SaveOptions, SaveReason};
use crate::vim::takes_keys;

impl Workbench {
    /// Runs the `:` command line `line` (the text after `:`) in the active
    /// code view.
    pub(crate) fn run_vim_command(
        &mut self,
        line: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_code_view(cx).filter(|v| takes_keys(v.read(cx))) else {
            return;
        };
        let vim = self.vim().clone();
        vim.update(cx, |vim, cx| vim.run_command_line(&view, line, cx));
        self.run_vim_requests(window, cx);
    }

    /// Runs what the last vim key or command asked for: a command id, ex
    /// commands, the `:` line.
    pub(crate) fn run_vim_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (command, ex, line) = self.vim().update(cx, |vim, _| {
            (vim.take_command(), vim.take_ex(), vim.take_command_line())
        });
        if let Some(command) = command {
            self.run_command(command, None, window, cx);
        }
        for cmd in ex {
            self.run_ex_command(cmd, window, cx);
        }
        if let Some(prefill) = line {
            self.show_quick_open(&format!(":{prefill}"), window, cx);
        }
    }

    fn run_ex_command(&mut self, cmd: ExCommand, window: &mut Window, cx: &mut Context<Self>) {
        let mut run = |wb: &mut Self, id: &str| {
            wb.run_command(id, None, window, cx);
        };
        match cmd {
            ExCommand::Save { overwrite: false } => run(self, "workbench.action.files.save"),
            ExCommand::SaveAll => run(self, "workbench.action.files.saveAll"),
            ExCommand::Close { revert: false } => run(self, "workbench.action.closeActiveEditor"),
            ExCommand::CloseAll => run(self, "workbench.action.closeAllEditors"),
            ExCommand::Save { overwrite: true } => {
                let Some((_, path)) = self.active_file_tab(cx) else {
                    return;
                };
                self.documents()
                    .clone()
                    .update(cx, |d, cx| d.save(&path, save_options(true), window, cx))
                    .detach();
            }
            ExCommand::Close { revert: true } => {
                let Some((id, path)) = self.active_file_tab(cx) else {
                    return;
                };
                let (documents, groups) = (self.documents().clone(), self.editor_groups().clone());
                if !documents.read(cx).is_dirty(&path) {
                    close_tab(&groups, id, cx);
                    return;
                }
                let revert = documents.update(cx, |d, cx| d.revert(&path, cx));
                cx.spawn(async move |_, cx| {
                    revert.await;
                    cx.update(|cx| close_tab(&groups, id, cx));
                })
                .detach();
            }
            ExCommand::SaveAndClose {
                only_if_dirty,
                overwrite,
            } => {
                let Some((id, path)) = self.active_file_tab(cx) else {
                    return;
                };
                let (documents, groups) = (self.documents().clone(), self.editor_groups().clone());
                if only_if_dirty && !documents.read(cx).is_dirty(&path) {
                    close_tab(&groups, id, cx);
                    return;
                }
                let save = documents.update(cx, |d, cx| {
                    d.save(&path, save_options(overwrite), window, cx)
                });
                cx.spawn(async move |_, cx| {
                    // A conflict answered with Cancel or Reload, a failed
                    // write, or an edit typed meanwhile keeps the tab.
                    let saved = save.await.is_ok() && !is_dirty(&documents, &path, cx);
                    if saved {
                        cx.update(|cx| close_tab(&groups, id, cx));
                    }
                })
                .detach();
            }
            // Vim clears the highlights itself.
            ExCommand::NoHighlight => {}
        }
    }

    /// The active tab and the path of the code view it shows.
    fn active_file_tab(&self, cx: &gpui::App) -> Option<(TabId, Arc<Path>)> {
        let view = self.active_code_view(cx)?;
        let id = self.editor_groups().read(cx).model().active_tab()?.id;
        Some((id, view.read(cx).path().clone()))
    }
}

/// An explicit save with the save participants; `overwrite` for `!`.
fn save_options(overwrite: bool) -> SaveOptions {
    SaveOptions {
        reason: SaveReason::Explicit,
        participants: true,
        overwrite,
    }
}

fn is_dirty(documents: &Entity<Documents>, path: &Path, cx: &mut gpui::AsyncApp) -> bool {
    documents.read_with(cx, |d, _| d.is_dirty(path))
}

/// Closes tab `id` without asking.
fn close_tab(groups: &Entity<EditorGroupsView>, id: TabId, cx: &mut gpui::App) {
    groups.update(cx, |view, cx| {
        view.update_model(cx, |m| m.close(id));
    });
}
