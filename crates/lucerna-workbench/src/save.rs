//! Saving (spec §6.1): the save options, the save participants, the write
//! itself (off the main thread) and the save and revert commands.
//!
//! The flow lives in [`crate::Documents::save`]: it asks about a conflict with the
//! disk, runs the participants as one undo step, captures the text and
//! writes it here in the background, then marks what it captured as saved.

use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use encoding_rs::Encoding;
use futures::channel::oneshot;
use gpui::{App, AsyncWindowContext, Context, Entity, PromptLevel, Subscription, Task, Window};
use lucerna_text::{Buffer, BufferSnapshot, LineEnding, Point, WriteError, Written, write_bytes};

use crate::app_state::AppState;
use crate::documents::{DocState, DocumentEvent, Edits, Stamp};
use crate::{CommandRegistry, Documents, Workbench};

/// Why a document is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveReason {
    /// A save command.
    Explicit,
    /// `files.autoSave`.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveOptions {
    pub reason: SaveReason,
    /// Run the save participants (`files.trimTrailingWhitespace`,
    /// `files.insertFinalNewline`, `files.trimFinalNewlines`).
    pub participants: bool,
}

/// The conflict prompt's message and buttons (spec §7.3).
pub(crate) const CONFLICT_MESSAGE: &str =
    "This file has changed on disk since you started editing it. Do you want to overwrite it?";
pub(crate) const CONFLICT_ANSWERS: [&str; 3] = ["Overwrite", "Reload", "Cancel"];

/// The detail of the close and quit prompts (spec §6.3).
pub(crate) const UNSAVED_DETAIL: &str = "Your changes will be lost if you don't save them.";
/// The close prompt's buttons; the quit prompt's with one file.
pub(crate) const CLOSE_ANSWERS: [&str; 3] = ["Save", "Don't Save", "Cancel"];
/// The quit prompt's buttons with several files.
const QUIT_ANSWERS: [&str; 3] = ["Save All", "Don't Save", "Cancel"];

/// What a save prompt's answer asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveChoice {
    Save,
    DontSave,
    /// Also a dismissed prompt.
    Cancel,
}

impl SaveChoice {
    /// The choice of the button at `answer` of [`CLOSE_ANSWERS`] or the
    /// quit prompt's buttons; `Err` (the prompt went away) is Cancel.
    pub(crate) fn of<E>(answer: Result<usize, E>) -> SaveChoice {
        match answer {
            Ok(0) => SaveChoice::Save,
            Ok(1) => SaveChoice::DontSave,
            _ => SaveChoice::Cancel,
        }
    }
}

/// The close prompt for a document named `name`, as VS Code words it.
pub(crate) fn prompt_unsaved(
    name: &str,
    window: &mut Window,
    cx: &mut App,
) -> oneshot::Receiver<usize> {
    let message = format!("Do you want to save the changes you made to {name}?");
    window.prompt(
        PromptLevel::Warning,
        &message,
        Some(UNSAVED_DETAIL),
        &CLOSE_ANSWERS[..],
        cx,
    )
}

/// The quit (or window close) prompt for the dirty documents named
/// `names`: one prompt listing them all; with one, the close prompt.
pub(crate) fn prompt_unsaved_all(
    names: &[String],
    window: &mut Window,
    cx: &mut App,
) -> oneshot::Receiver<usize> {
    if let [name] = names {
        return prompt_unsaved(name, window, cx);
    }
    let message = format!(
        "Do you want to save the changes to the following {} files?",
        names.len()
    );
    let detail = format!("{}\n\n{UNSAVED_DETAIL}", names.join("\n"));
    window.prompt(
        PromptLevel::Warning,
        &message,
        Some(&detail),
        &QUIT_ANSWERS[..],
        cx,
    )
}

/// The file name of `path`, as prompts and messages name a document.
pub(crate) fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

/// Saves the documents at `paths` (explicitly, with participants) one
/// after another, each of which may ask about a conflict. `true` when
/// every one was saved; a failure is reported in the status bar and the
/// rest still save.
pub(crate) async fn save_each(
    documents: Entity<Documents>,
    paths: Vec<Arc<Path>>,
    cx: &mut AsyncWindowContext,
) -> bool {
    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: true,
    };
    let mut all = true;
    for path in paths {
        let Ok(save) =
            cx.update(|window, cx| documents.update(cx, |d, cx| d.save(&path, opts, window, cx)))
        else {
            return false;
        };
        all &= matches!(save.await, Ok(true));
    }
    all
}

/// A save participant: the edits it makes to `text`.
/// `cursor_rows` (sorted) are lines trimming must leave alone.
pub(crate) type Participant = fn(&BufferSnapshot, &[u32]) -> Edits;

/// The participants enabled for a document of `language`, in the order
/// they run.
pub(crate) fn participants(
    settings: &lucerna_settings::SettingsStore,
    language: Option<&str>,
    reason: SaveReason,
) -> Vec<Participant> {
    let on = |key: &str| settings.get::<bool>(key, language).unwrap_or(false);
    let mut out: Vec<Participant> = Vec::new();
    if on("files.trimTrailingWhitespace") {
        out.push(trim_trailing_whitespace);
    }
    // 4.6: format runs here, never for Auto.
    if reason != SaveReason::Auto && on("editor.formatOnSave") {
        // No formatter yet: the slot is wired in step 4.6.
    }
    if on("files.insertFinalNewline") {
        out.push(insert_final_newline);
    }
    if on("files.trimFinalNewlines") {
        out.push(trim_final_newlines);
    }
    out
}

/// Deletes the spaces and tabs at the end of every line (VS Code trims
/// those two only).
fn trim_trailing_whitespace(text: &BufferSnapshot, cursor_rows: &[u32]) -> Edits {
    let mut edits = Vec::new();
    for row in 0..text.line_count() {
        if cursor_rows.binary_search(&row).is_ok() {
            continue;
        }
        let line = text.line(row);
        let kept = line.trim_end_matches([' ', '\t']).len();
        if kept < line.len() {
            let start = text.point_to_offset(Point {
                row,
                column: kept as u32,
            });
            edits.push((start..start + (line.len() - kept), Arc::from("")));
        }
    }
    edits
}

/// Appends a line break to text that does not end with one.
fn insert_final_newline(text: &BufferSnapshot, cursor_rows: &[u32]) -> Edits {
    let len = text.len_bytes();
    if len == 0 || text.byte_at(len - 1) == b'\n' {
        return Vec::new();
    }
    // An autosave must not push a caret at the end past the new line.
    if cursor_rows.contains(&text.offset_to_point(len).row) {
        return Vec::new();
    }
    vec![(len..len, Arc::from("\n"))]
}

/// Collapses the line breaks at the end of the text to one.
fn trim_final_newlines(text: &BufferSnapshot, cursor_rows: &[u32]) -> Edits {
    let len = text.len_bytes();
    let mut start = len;
    while start > 0 && text.byte_at(start - 1) == b'\n' {
        start -= 1;
    }
    // An autosave keeps the blank lines that hold a cursor.
    let kept_row = text.offset_to_point(start).row;
    if len - start > 1 && !cursor_rows.iter().any(|&row| row > kept_row) {
        vec![(start + 1..len, Arc::from(""))]
    } else {
        Vec::new()
    }
}

/// How the captured text is stored: copied off the buffer, so the write
/// can run on another thread while the buffer stays on the main one.
pub(crate) struct WriteJob {
    pub(crate) snapshot: BufferSnapshot,
    pub(crate) encoding: &'static Encoding,
    pub(crate) has_bom: bool,
    pub(crate) line_ending: LineEnding,
    /// The stamp and hash of the file as last loaded or saved. When set,
    /// a file that no longer matches them is not written over.
    pub(crate) expected: Option<(Option<Stamp>, u64)>,
}

/// What a write ended in.
pub(crate) enum WriteOutcome {
    /// The written bytes' hash and the file's metadata after the write.
    Written(Written, Option<fs::Metadata>),
    /// Another program changed the file since it was last loaded or saved;
    /// nothing was written.
    ChangedOnDisk,
}

impl WriteJob {
    /// Encodes and writes the text to `path`.
    pub(crate) fn run(self, path: &Path) -> Result<WriteOutcome, WriteError> {
        if let Some((stamp, hash)) = self.expected
            && changed_on_disk(path, stamp, hash)?
        {
            return Ok(WriteOutcome::ChangedOnDisk);
        }
        let bytes = Buffer::encode(
            &self.snapshot,
            self.encoding,
            self.has_bom,
            self.line_ending,
        )?;
        let written = write_bytes(path, &bytes)?;
        Ok(WriteOutcome::Written(written, fs::metadata(path).ok()))
    }
}

/// Whether the file at `path` differs from the one with `stamp` and
/// `hash`, the way VS Code checks a file's etag before a save: an equal
/// stamp is trusted, otherwise the bytes are hashed. A missing file has
/// not changed (the save makes it again).
fn changed_on_disk(path: &Path, stamp: Option<Stamp>, hash: u64) -> Result<bool, WriteError> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if stamp == Some(Stamp::of(&meta)) {
        return Ok(false);
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    Ok(hash_bytes(&bytes) != hash)
}

/// The hash of a file's bytes, as `decode_file` and `write_bytes` hash
/// them.
pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// The message for a failed save of `path`.
pub(crate) fn failure_message(path: &Path, err: &WriteError) -> String {
    format!("Failed to save '{}': {err}", display_name(path))
}

/// The file the save and revert commands act on: the active code view's,
/// or the file a focused Markdown preview (static or dynamic) renders
/// (spec §4.1).
fn active_document_path(wb: &Workbench, cx: &App) -> Option<Arc<Path>> {
    if let Some(view) = wb.active_code_view(cx) {
        return Some(view.read(cx).path().clone());
    }
    let groups = wb.editor_groups().read(cx);
    let tab = groups.model().active_tab()?;
    tab.path().or(tab.previewed_path()).cloned()
}

/// Saves the document of the active code view or preview.
fn save_active(
    wb: &mut Workbench,
    opts: SaveOptions,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(path) = active_document_path(wb, cx) else {
        return;
    };
    wb.documents()
        .clone()
        .update(cx, |d, cx| d.save(&path, opts, window, cx))
        .detach();
}

/// Saves every dirty document, one after another (each may ask about a
/// conflict).
fn save_all(wb: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>) {
    let documents = wb.documents().clone();
    let paths = documents.read(cx).dirty_paths();
    cx.spawn_in(window, async move |_, cx| {
        save_each(documents, paths, cx).await;
    })
    .detach();
}

/// `files.autoSave` (spec §6.2): saves dirty documents on its own, never
/// asking and never touching a conflicted or deleted one.
#[derive(Default)]
pub(crate) struct AutoSave {
    /// The `afterDelay` timer of each document with unsaved changes;
    /// dropping one cancels it.
    timers: HashMap<Arc<Path>, Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// The default of `files.autoSaveDelay`, in milliseconds.
const DEFAULT_AUTO_SAVE_DELAY: u64 = 1000;

impl Workbench {
    /// Starts following `documents` and the window for `files.autoSave`.
    pub(crate) fn start_auto_save(
        &mut self,
        documents: &Entity<Documents>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = cx.subscribe_in(documents, window, Self::auto_save_event);
        let activation = cx.observe_window_activation(window, |wb, window, cx| {
            if !window.is_window_active() {
                wb.auto_save_where("onWindowChange", window, cx);
            }
        });
        self.auto_save._subscriptions = vec![events, activation];
    }

    /// `files.autoSave` for the document at `path`.
    fn auto_save_mode(&self, path: &Path, cx: &App) -> String {
        let language = self
            .documents()
            .read(cx)
            .language_id_of(path)
            .map(|id| id.to_string());
        cx.global::<AppState>()
            .settings
            .get::<String>("files.autoSave", language.as_deref())
            .unwrap_or_else(|| "off".into())
    }

    fn auto_save_delay(&self, path: &Path, cx: &App) -> Duration {
        let language = self
            .documents()
            .read(cx)
            .language_id_of(path)
            .map(|id| id.to_string());
        let ms = cx
            .global::<AppState>()
            .settings
            .get::<u64>("files.autoSaveDelay", language.as_deref())
            .unwrap_or(DEFAULT_AUTO_SAVE_DELAY);
        Duration::from_millis(ms)
    }

    fn auto_save_event(
        &mut self,
        documents: &Entity<Documents>,
        event: &DocumentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Only a renamed document opens with unsaved changes: its timer
            // went with `Closed` at the old path.
            DocumentEvent::Opened { path, .. }
                if documents.read(cx).is_dirty(path)
                    && self.auto_save_mode(path, cx) == "afterDelay" =>
            {
                self.arm_auto_save(path, window, cx);
            }
            DocumentEvent::Edited { path, .. } => {
                if self.auto_save_mode(path, cx) != "afterDelay" {
                    self.auto_save.timers.remove(path);
                    return;
                }
                // Restarted by every edit: the old timer is dropped.
                self.arm_auto_save(path, window, cx);
            }
            DocumentEvent::DirtyChanged { path, dirty: false } | DocumentEvent::Closed { path } => {
                self.auto_save.timers.remove(path);
            }
            DocumentEvent::FocusLost { path }
                if self.auto_save_mode(path, cx) == "onFocusChange" =>
            {
                self.auto_save_document(path, window, cx);
            }
            _ => {}
        }
    }

    /// Autosaves the document at `path` once `files.autoSaveDelay` has
    /// passed, replacing its timer.
    fn arm_auto_save(&mut self, path: &Arc<Path>, window: &mut Window, cx: &mut Context<Self>) {
        let delay = self.auto_save_delay(path, cx);
        let target = path.clone();
        let timer = cx.spawn_in(window, async move |wb, cx| {
            cx.background_executor().timer(delay).await;
            wb.update_in(cx, |wb, window, cx| {
                wb.auto_save.timers.remove(&target);
                wb.auto_save_document(&target, window, cx);
            })
            .ok();
        });
        self.auto_save.timers.insert(path.clone(), timer);
    }

    /// Autosaves every dirty document whose `files.autoSave` is `mode`.
    fn auto_save_where(&mut self, mode: &str, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.documents().read(cx).dirty_paths();
        for path in paths {
            if self.auto_save_mode(&path, cx) == mode {
                self.auto_save_document(&path, window, cx);
            }
        }
    }

    /// Saves the document at `path` if it has unsaved changes and is
    /// neither in conflict with the disk nor deleted. While a view of it is
    /// in the middle of an IME composition, the save waits another
    /// `files.autoSaveDelay`: its participants' edits and its undo stop
    /// would split the composition.
    fn auto_save_document(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let documents = self.documents().clone();
        {
            let docs = documents.read(cx);
            if !docs.is_dirty(path)
                || docs.has_conflict(path)
                || docs.state(path) != Some(&DocState::Ready)
            {
                return;
            }
            if docs
                .views_of(path)
                .iter()
                .any(|v| v.read(cx).is_composing())
            {
                self.arm_auto_save(&Arc::from(path), window, cx);
                return;
            }
        }
        let opts = SaveOptions {
            reason: SaveReason::Auto,
            participants: true,
        };
        documents
            .update(cx, |d, cx| d.save(path, opts, window, cx))
            .detach();
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.files.save",
        "File: Save",
        |wb, _, window, cx| {
            let opts = SaveOptions {
                reason: SaveReason::Explicit,
                participants: true,
            };
            save_active(wb, opts, window, cx)
        },
    );
    commands.register(
        "workbench.action.files.saveAll",
        "File: Save All Files",
        |wb, _, window, cx| save_all(wb, window, cx),
    );
    commands.register(
        "workbench.action.files.saveWithoutFormatting",
        "File: Save without Formatting",
        |wb, _, window, cx| {
            let opts = SaveOptions {
                reason: SaveReason::Explicit,
                participants: false,
            };
            save_active(wb, opts, window, cx)
        },
    );
    commands.register(
        "workbench.action.files.revert",
        "File: Revert File",
        |wb, _, _, cx| {
            let Some(path) = active_document_path(wb, cx) else {
                return;
            };
            wb.documents()
                .clone()
                .update(cx, |d, cx| d.revert(&path, cx))
                .detach();
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, f: Participant) -> String {
        let buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
        let snapshot = buffer.snapshot();
        let mut out = text.to_string();
        for (range, new) in f(&snapshot, &[]).into_iter().rev() {
            out.replace_range(range, &new);
        }
        out
    }

    #[test]
    fn trims_spaces_and_tabs_only() {
        assert_eq!(
            apply("a \t\nb\u{a0} \n  \nc ", trim_trailing_whitespace),
            "a\nb\u{a0}\n\nc"
        );
        assert_eq!(apply("", trim_trailing_whitespace), "");
    }

    #[test]
    fn final_newline_is_added_to_non_empty_text() {
        assert_eq!(apply("a", insert_final_newline), "a\n");
        assert_eq!(apply("a\n", insert_final_newline), "a\n");
        assert_eq!(apply("", insert_final_newline), "");
    }

    #[test]
    fn final_newlines_collapse_to_one() {
        assert_eq!(apply("a\n\n\n", trim_final_newlines), "a\n");
        assert_eq!(apply("a\n", trim_final_newlines), "a\n");
        assert_eq!(apply("a", trim_final_newlines), "a");
        assert_eq!(apply("\n\n", trim_final_newlines), "\n");
    }

    #[test]
    fn auto_tail_participants_leave_cursors_alone() {
        let run = |text: &str, f: Participant, rows: &[u32]| {
            let buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
            f(&buffer.snapshot(), rows).len()
        };
        assert_eq!(run("a", insert_final_newline, &[0]), 0);
        assert_eq!(run("a\nb", insert_final_newline, &[0]), 1);
        assert_eq!(run("a\n\n\n", trim_final_newlines, &[2]), 0);
        assert_eq!(run("a\n\n\n", trim_final_newlines, &[0]), 1);
    }
}
