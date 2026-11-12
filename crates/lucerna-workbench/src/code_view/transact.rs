//! Edits from the view: [`CodeView::transact`] applies an [`EditPlan`] to
//! the document with VS Code's undo stops (spec §4.1, §4.2), and the edit,
//! clipboard, undo and `cursorUndo` commands build on it.

use std::sync::Arc;

use gpui::{App, ClipboardItem, Context, WeakEntity};
use lucerna_editor::clipboard::{self, ClipboardText};
use lucerna_editor::{Ctx, EditConfig, EditKind, EditPlan, SelectionSet, should_stop_between};
use lucerna_text::SelectionsToken;
use serde::{Deserialize, Serialize};

use super::{CodeView, DEFAULT_WORD_SEPARATORS, SelectionChange};
use crate::AppState;
use crate::documents::{DocState, ViewUpdate};

/// The `ClipboardItem` metadata of a copy: the pieces of a copy from
/// several selections, and whether it is a whole line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardMetadata {
    pieces: Option<Vec<String>>,
    whole_line: bool,
}

impl CodeView {
    /// The view does not take edits: its file failed to load, or it is a
    /// picker's preview (`editorReadonly`).
    pub fn is_readonly(&self) -> bool {
        !matches!(self.doc_state, DocState::Ready | DocState::Deleted) || self.passive
    }

    /// Whether typing and edit keys change the text: not read-only, and no
    /// cursor overlay (vim's Normal-mode cursor) is shown.
    pub(crate) fn takes_text(&self) -> bool {
        !self.is_readonly() && self.cursor_overlay.is_none()
    }

    /// The editing settings of this view's language.
    pub fn edit_config(&self, cx: &App) -> EditConfig {
        let settings = &cx.global::<AppState>().settings;
        let language = self.language_id.as_deref();
        EditConfig {
            tab_size: self.config.tab_size,
            insert_spaces: settings
                .get("editor.insertSpaces", language)
                .unwrap_or(true),
            // `editor.autoIndent` arrives in 4.2; until then, always keep.
            keep_indent: true,
            trim_auto_whitespace: settings
                .get("editor.trimAutoWhitespace", language)
                .unwrap_or(true),
            word_separators: settings
                .get::<String>("editor.wordSeparators", language)
                .map_or_else(|| Arc::from(DEFAULT_WORD_SEPARATORS), Arc::from),
            empty_selection_clipboard: settings
                .get("editor.emptySelectionClipboard", language)
                .unwrap_or(true),
        }
    }

    /// Applies `plan` to the document as part of the current undo step, or
    /// a new one: a stop goes before it when `plan.stop_before` or the
    /// kind changed (`should_stop_between`), and after it when
    /// `plan.stop_after`. The selections become the plan's (no cursor
    /// move) and the primary scrolls into view. Does nothing in a
    /// read-only view; returns whether the edit was applied.
    pub fn transact(&mut self, plan: EditPlan, cx: &mut Context<Self>) -> bool {
        if self.is_readonly() {
            return false;
        }
        let Some(documents) = self.documents.as_ref().and_then(WeakEntity::upgrade) else {
            return false;
        };
        let stop = plan.stop_before || should_stop_between(self.last_kind, plan.kind);
        let before: SelectionsToken = Arc::new(self.selections.clone());
        let after: SelectionsToken = Arc::new(plan.selections.clone());
        let (path, view) = (self.path.clone(), cx.entity_id());
        let update = documents.update(cx, |docs, cx| {
            if stop {
                docs.push_stop(&path);
            }
            let update =
                docs.apply_edits_from_view(&path, plan.edits, Some(before), Some(after), view, cx);
            if update.is_some() && plan.stop_after {
                docs.push_stop(&path);
            }
            update
        });
        let Some(update) = update else {
            return false;
        };
        self.take_update(update, cx);
        self.set_selections_without_stop(plan.selections, true, cx);
        self.last_kind = plan.kind;
        true
    }

    /// Shows a change this view made (see `Documents::apply_edits_from_view`).
    fn take_update(&mut self, update: ViewUpdate, cx: &mut Context<Self>) {
        if update.patch.is_empty() {
            return;
        }
        // The document's syntax replaces the view's: no need to edit it.
        self.syntax = None;
        self.apply_edit(&update.old, update.snapshot, &update.patch, cx);
        self.set_syntax(update.syntax, cx);
    }

    /// Runs the edit command `f` on the selections when the view takes
    /// text.
    pub(crate) fn edit(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(Ctx, &SelectionSet) -> EditPlan,
    ) -> bool {
        if !self.takes_text() {
            return false;
        }
        let config = self.edit_config(cx);
        let ctx = Ctx {
            snapshot: &self.snapshot,
            config: &config,
        };
        let plan = f(ctx, &self.selections);
        self.transact(plan, cx)
    }

    /// Types `text` at every selection (the `type` command): `\n` is
    /// Enter, which keeps the indentation.
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        self.unmark();
        let prev = self.last_kind;
        if text == "\n" {
            self.edit(cx, lucerna_editor::commands::newline)
        } else {
            self.edit(cx, |ctx, set| {
                lucerna_editor::commands::insert(ctx, set, text, prev)
            })
        }
    }

    /// Ends the IME composition: the composed text stays as it is.
    pub(crate) fn unmark(&mut self) {
        self.marked = None;
    }

    /// Undoes (or, without `undo`, redoes) the document's last step and
    /// restores the selections it had, without an undo stop. Ends any
    /// composition first. Returns whether there was a step.
    pub fn undo(&mut self, undo: bool, cx: &mut Context<Self>) -> bool {
        self.unmark();
        if self.is_readonly() {
            return false;
        }
        let Some(documents) = self.documents.as_ref().and_then(WeakEntity::upgrade) else {
            return false;
        };
        let (path, view) = (self.path.clone(), cx.entity_id());
        let Some((update, selections)) =
            documents.update(cx, |docs, cx| docs.undo_from_view(&path, undo, view, cx))
        else {
            return false;
        };
        self.take_update(update, cx);
        let restored = selections.and_then(|s| s.downcast_ref::<SelectionSet>().cloned());
        if let Some(mut set) = restored {
            set.clamp(&self.snapshot);
            self.set_selections_without_stop(set, true, cx);
        }
        self.last_kind = EditKind::Other;
        true
    }

    /// `cursorUndo` (or, without `undo`, `cursorRedo`): back to the
    /// selections before the last cursor move.
    pub fn cursor_undo(&mut self, undo: bool, cx: &mut Context<Self>) {
        let popped = if undo {
            self.cursor_undo.pop_back()
        } else {
            self.cursor_redo.pop()
        };
        let Some(set) = popped else {
            return;
        };
        let current = self.selections.clone();
        if undo {
            self.cursor_redo.push(current);
        } else {
            self.cursor_undo.push_back(current);
        }
        self.set_goto_message(None, cx);
        self.block_selection = None;
        self.set_selections_inner(set, true, SelectionChange::History, cx);
    }

    /// The text to copy: the block selection's rows, or the selections
    /// (a whole line for an empty one, with `editor.emptySelectionClipboard`).
    fn clipboard_text(&self, config: &EditConfig) -> Option<ClipboardText> {
        if self.block_selection.is_some() {
            return self.copy_selection().map(|text| ClipboardText {
                text,
                ..ClipboardText::default()
            });
        }
        let ctx = Ctx {
            snapshot: &self.snapshot,
            config,
        };
        let clip = clipboard::copy(ctx, &self.selections);
        (!clip.text.is_empty()).then_some(clip)
    }

    /// `editor.action.clipboardCopyAction`.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let config = self.edit_config(cx);
        if let Some(clip) = self.clipboard_text(&config) {
            cx.write_to_clipboard(clipboard_item(clip));
        }
    }

    /// `editor.action.clipboardCutAction`.
    pub fn cut(&mut self, cx: &mut Context<Self>) {
        if !self.takes_text() {
            return self.copy(cx);
        }
        let config = self.edit_config(cx);
        let ctx = Ctx {
            snapshot: &self.snapshot,
            config: &config,
        };
        let (plan, clip) = clipboard::cut(ctx, &self.selections);
        if clip.text.is_empty() {
            return;
        }
        cx.write_to_clipboard(clipboard_item(clip));
        self.transact(plan, cx);
    }

    /// `editor.action.clipboardPasteAction`.
    pub fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text() else {
            return;
        };
        let meta = item
            .metadata()
            .and_then(|m| serde_json::from_str::<ClipboardMetadata>(m).ok());
        let clip = ClipboardText {
            text,
            pieces: meta.as_ref().and_then(|m| m.pieces.clone()),
            whole_line: meta.is_some_and(|m| m.whole_line),
        };
        self.unmark();
        self.edit(cx, |ctx, set| clipboard::paste(ctx, set, &clip));
    }
}

/// The clipboard item of `clip`: its text, with the pieces and the
/// whole-line flag as JSON metadata.
fn clipboard_item(clip: ClipboardText) -> ClipboardItem {
    let meta = ClipboardMetadata {
        pieces: clip.pieces,
        whole_line: clip.whole_line,
    };
    ClipboardItem::new_string_with_json_metadata(clip.text, meta)
}
