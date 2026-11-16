//! Edits from the view: [`CodeView::transact`] applies an [`EditPlan`] to
//! the document with VS Code's undo stops (spec §4.1, §4.2), and the edit,
//! clipboard, undo and `cursorUndo` commands build on it.

use std::sync::Arc;

use gpui::{App, ClipboardItem, Context, WeakEntity};
use lucerna_editor::clipboard::{self, ClipboardText};
use lucerna_editor::{
    AutoClosing, AutoClosingEdit, AutoIndent, AutoSurround, Ctx, EditConfig, EditKind, EditPlan,
    SelectionSet, should_stop_between,
};
use lucerna_syntax::Language;
use lucerna_text::SelectionsToken;
use lucerna_vim::Typed;
use serde::{Deserialize, Serialize};

use super::{CodeView, DEFAULT_WORD_SEPARATORS, SelectionChange};
use crate::documents::{DocState, ViewUpdate};
use crate::tasks::catch_panic;

/// The `ClipboardItem` metadata of a copy: the pieces of a copy from
/// several selections, and whether it is a whole line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardMetadata {
    pub(crate) pieces: Option<Vec<String>>,
    pub(crate) whole_line: bool,
}

impl CodeView {
    /// The view does not take edits: its file failed to load, or it is a
    /// picker's preview (`editorReadonly`).
    pub fn is_readonly(&self) -> bool {
        !matches!(self.doc_state, DocState::Ready | DocState::Deleted) || self.passive
    }

    /// Whether typing and edit keys change the text: not read-only, and no
    /// block cursor (vim's Normal mode) is shown.
    pub(crate) fn takes_text(&self) -> bool {
        !self.is_readonly() && self.block_cursor().is_none()
    }

    /// The editing settings of this view's language.
    pub fn edit_config(&self, cx: &App) -> EditConfig {
        let settings = self.settings.read(cx).view(cx);
        let language = self.language_id.as_deref();
        EditConfig {
            tab_size: self.config.tab_size,
            insert_spaces: self.config.insert_spaces,
            auto_indent: setting_enum(settings, "editor.autoIndent", language, AutoIndent::parse)
                .unwrap_or(AutoIndent::Full),
            auto_closing_brackets: setting_enum(
                settings,
                "editor.autoClosingBrackets",
                language,
                AutoClosing::parse,
            )
            .unwrap_or(AutoClosing::LanguageDefined),
            auto_closing_quotes: setting_enum(
                settings,
                "editor.autoClosingQuotes",
                language,
                AutoClosing::parse,
            )
            .unwrap_or(AutoClosing::LanguageDefined),
            auto_closing_overtype: setting_enum(
                settings,
                "editor.autoClosingOvertype",
                language,
                AutoClosingEdit::parse,
            )
            .unwrap_or(AutoClosingEdit::Auto),
            auto_closing_delete: setting_enum(
                settings,
                "editor.autoClosingDelete",
                language,
                AutoClosingEdit::parse,
            )
            .unwrap_or(AutoClosingEdit::Auto),
            auto_surround: setting_enum(
                settings,
                "editor.autoSurround",
                language,
                AutoSurround::parse,
            )
            .unwrap_or(AutoSurround::LanguageDefined),
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

    /// The document's language, for its editing data: the one its syntax
    /// parsed with, else the registry's entry for the view's language id.
    pub fn edit_language(&self, cx: &App) -> Option<Arc<Language>> {
        if let Some(syntax) = &self.syntax {
            return Some(syntax.language().clone());
        }
        let id = self.language_id.as_deref()?;
        let languages = self.settings().read(cx).languages(cx);
        languages.languages().iter().find(|l| &*l.id == id).cloned()
    }

    /// Applies `plan` to the document as part of the current undo step, or
    /// a new one: a stop goes before it when `plan.stop_before` or the
    /// kind changed (`should_stop_between`), and after it when
    /// `plan.stop_after`. The selections become the plan's (no cursor
    /// move) and the primary scrolls into view. During vim's Insert
    /// session (`suppress_stops`) no stop is pushed. Does nothing in a
    /// read-only view; returns whether the edit was applied.
    pub fn transact(&mut self, plan: EditPlan, cx: &mut Context<Self>) -> bool {
        if self.is_readonly() {
            return false;
        }
        let Some(documents) = self.documents.as_ref().and_then(WeakEntity::upgrade) else {
            return false;
        };
        let suppress = self.suppress_stops;
        let stop =
            !suppress && (plan.stop_before || should_stop_between(self.last_kind, plan.kind));
        if suppress && let Some(flag) = &self.stop_requested {
            // A cursor move asked for a stop before the session held them.
            flag.set(false);
        }
        let before: SelectionsToken = Arc::new(self.selections.clone());
        let after: SelectionsToken = Arc::new(plan.selections.clone());
        let (path, view) = (self.path.clone(), cx.entity_id());
        let update = documents.update(cx, |docs, cx| {
            if stop {
                docs.push_stop(&path);
            }
            let update =
                docs.apply_edits_from_view(&path, plan.edits, Some(before), Some(after), view, cx);
            if update.is_some() && plan.stop_after && !suppress {
                docs.push_stop(&path);
            }
            update
        });
        let Some(update) = update else {
            return false;
        };
        self.break_record();
        self.take_update(update, cx);
        // The new pairs, in the new text; the selections keep those they
        // are inside.
        self.autoclose_regions.extend(plan.autoclose);
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

    /// Runs `f`, which builds an edit or moves the cursors, so that a
    /// panic in it cannot take the app down (spec §16): it is logged,
    /// nothing is applied (`None`), the composition ends and the undo step
    /// is closed.
    pub(crate) fn contained<T>(
        &mut self,
        name: &'static str,
        f: impl FnOnce(&Self) -> T,
        cx: &mut Context<Self>,
    ) -> Option<T> {
        match catch_panic(name, || f(self)) {
            Ok(value) => Some(value),
            Err(_) => {
                self.unmark();
                if let Some(documents) = self.documents.as_ref().and_then(WeakEntity::upgrade) {
                    let path = self.path.clone();
                    documents.update(cx, |docs, _| docs.push_stop(&path));
                }
                cx.notify();
                None
            }
        }
    }

    /// Runs the edit command `f` on the selections when the view takes
    /// text. A panic in `f` applies nothing (see `contained`). Returns
    /// whether an edit was applied.
    pub fn edit(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(Ctx, &SelectionSet) -> EditPlan,
    ) -> bool {
        if !self.takes_text() {
            return false;
        }
        let config = self.edit_config(cx);
        let language = self.edit_language(cx);
        let plan = self.contained(
            "edit command",
            |v| {
                let ctx = Ctx {
                    snapshot: &v.snapshot,
                    config: &config,
                    language: language.as_deref().map(Language::editing),
                    syntax: v.syntax.as_deref(),
                };
                f(ctx, &v.selections)
            },
            cx,
        );
        plan.is_some_and(|plan| self.transact(plan, cx))
    }

    /// Types `text` at every selection (the `type` command): `\n` is
    /// Enter, which keeps the indentation.
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        self.unmark();
        let prev = self.last_kind;
        let applied = self.typing(|v| {
            if text == "\n" {
                v.edit(cx, lucerna_editor::commands::newline)
            } else {
                let regions = v.autoclose_regions.clone();
                v.edit(cx, |ctx, set| {
                    lucerna_editor::commands::insert(ctx, set, text, prev, &regions)
                })
            }
        });
        if applied {
            let typed = if text == "\n" {
                Typed::Newline
            } else {
                Typed::Text(text.to_string())
            };
            self.emit_typed(typed, cx);
        }
        applied
    }

    /// `deleteLeft`: an empty auto-closed pair goes as a whole.
    pub fn delete_left(&mut self, cx: &mut Context<Self>) -> bool {
        self.unmark();
        let regions = self.autoclose_regions.clone();
        let applied = self.typing(|v| {
            v.edit(cx, |ctx, set| {
                lucerna_editor::commands::delete_left(ctx, set, &regions)
            })
        });
        if applied {
            self.emit_typed(Typed::DeleteLeft, cx);
        }
        applied
    }

    /// The `tab` command: indent, or insert a tab or spaces.
    pub fn tab(&mut self, cx: &mut Context<Self>) -> bool {
        self.unmark();
        let applied = self.typing(|v| v.edit(cx, lucerna_editor::commands::tab));
        if applied {
            self.emit_typed(Typed::Tab, cx);
        }
        applied
    }

    /// Whether an IME composition is in progress.
    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
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
        // An undo during the Insert session is not typing.
        self.break_record();
        self.take_update(update, cx);
        self.autoclose_regions.clear();
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
            language: None,
            syntax: None,
        };
        let clip = clipboard::copy(ctx, &self.selections);
        (!clip.text.is_empty()).then_some(clip)
    }

    /// `editor.action.clipboardCopyAction`.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let config = self.edit_config(cx);
        let clip = self.contained("copy", |v| v.clipboard_text(&config), cx);
        if let Some(clip) = clip.flatten() {
            cx.write_to_clipboard(clipboard_item(clip));
        }
    }

    /// `editor.action.clipboardCutAction`.
    pub fn cut(&mut self, cx: &mut Context<Self>) {
        if !self.takes_text() {
            return self.copy(cx);
        }
        let config = self.edit_config(cx);
        let language = self.edit_language(cx);
        let cut = self.contained(
            "cut",
            |v| {
                let ctx = Ctx {
                    snapshot: &v.snapshot,
                    config: &config,
                    language: language.as_deref().map(Language::editing),
                    syntax: v.syntax.as_deref(),
                };
                clipboard::cut(ctx, &v.selections)
            },
            cx,
        );
        let Some((plan, clip)) = cut else {
            return;
        };
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
pub(crate) fn clipboard_item(clip: ClipboardText) -> ClipboardItem {
    let meta = ClipboardMetadata {
        pieces: clip.pieces,
        whole_line: clip.whole_line,
    };
    ClipboardItem::new_string_with_json_metadata(clip.text, meta)
}

/// A string setting parsed into an enum; `None` when unset or unknown, so
/// the caller falls back to the default.
fn setting_enum<T>(
    settings: lucerna_settings::SettingsView<'_>,
    key: &str,
    language: Option<&str>,
    parse: fn(&str) -> Option<T>,
) -> Option<T> {
    parse(&settings.get::<String>(key, language)?)
}
