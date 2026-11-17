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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use encoding_rs::Encoding;
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::Shared;
use gpui::{
    App, AsyncWindowContext, Context, Entity, Global, PromptLevel, Subscription, Task, Window,
};
use lucerna_text::{
    Buffer, BufferSnapshot, LineEnding, Point, TempFiles, WriteError, Written, write_bytes_tracked,
};

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
    /// Write over a file that changed on disk without asking or checking
    /// (`:w!`).
    pub overwrite: bool,
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

/// The buttons of the prompt before the first save of a text with invalid
/// bytes (spec §6.4). Cancel comes first: GPUI on macOS gives Return to the
/// first button, and Cancel is the default.
const LOSSY_ANSWERS: [&str; 2] = ["Cancel", "Save"];
/// The index of Save in [`LOSSY_ANSWERS`]: the only answer that agrees.
pub(crate) const LOSSY_SAVE: usize = 1;

/// The prompt before the first save of the document named `name`, whose
/// file had bytes that aren't valid `encoding` (spec §6.4). Only
/// [`LOSSY_SAVE`] agrees; any other answer, or a dismissed prompt, is
/// Cancel.
pub(crate) fn prompt_lossy(
    name: &str,
    encoding: &str,
    window: &mut Window,
    cx: &mut App,
) -> oneshot::Receiver<usize> {
    let message = format!("Save '{name}' and replace invalid bytes?");
    let detail = format!(
        "Bytes that aren't valid {encoding} will be written as \u{FFFD}. This can't be undone."
    );
    window.prompt(
        PromptLevel::Warning,
        &message,
        Some(&detail),
        &LOSSY_ANSWERS[..],
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
        overwrite: false,
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
    settings: lucerna_settings::SettingsView<'_>,
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

/// Removes the blank lines at the end of the text, as VS Code's
/// `TrimFinalNewLinesParticipant` does (MIT): a line with only spaces still
/// counts as content, one final line break stays after the last such line,
/// and a text of only blank lines becomes empty. An autosave keeps every
/// line up to the furthest cursor.
fn trim_final_newlines(text: &BufferSnapshot, cursor_rows: &[u32]) -> Edits {
    let len = text.len_bytes();
    // Rows that hold text; a final line break does not start another row.
    let rows = text.line_count();
    let ends_with_newline = len > 0 && text.byte_at(len - 1) == b'\n';
    // VS Code counts the empty line after a final line break.
    if rows + u32::from(ends_with_newline) <= 1 {
        return Vec::new();
    }
    let last_content = (0..rows).rev().find(|&row| !text.line(row).is_empty());
    let after_content = last_content.map_or(0, |row| row + 1);
    let after_cursor = cursor_rows.iter().max().map_or(0, |&row| row + 1);
    let keep = after_content.max(after_cursor);
    if keep >= rows {
        return Vec::new();
    }
    let start = text.point_to_offset(Point {
        row: keep,
        column: 0,
    });
    vec![(start..len, Arc::from(""))]
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
    /// An autosave: a file that is gone (deleted or moved before the
    /// watcher reported it) is not made again; a save command does.
    pub(crate) skip_if_missing: bool,
    /// Where the write's temp file is kept while it exists.
    pub(crate) temps: TempFiles,
}

/// What a write ended in.
pub(crate) enum WriteOutcome {
    /// The written bytes' hash, and the written file's size and mtime.
    Written(Written),
    /// Another program changed the file since it was last loaded or saved;
    /// nothing was written.
    ChangedOnDisk,
    /// An autosave found the file gone; nothing was written.
    Missing,
}

impl WriteJob {
    /// Encodes and writes the text to `path`.
    pub(crate) fn run(self, path: &Path) -> Result<WriteOutcome, WriteError> {
        if self.skip_if_missing
            && matches!(fs::metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
        {
            return Ok(WriteOutcome::Missing);
        }
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
        Ok(WriteOutcome::Written(write_bytes_tracked(
            path,
            &bytes,
            &self.temps,
        )?))
    }
}

/// The temp files of the app's saves in flight.
struct SaveTemps(TempFiles);

impl Global for SaveTemps {}

/// The temp files of every save in flight, shared by all windows. The
/// first call also deletes them when the app quits by a way that runs only
/// the `on_app_quit` hooks (Dock Quit, Log Out): those give a hook too
/// little time to wait for the writes.
pub fn save_temps(cx: &mut App) -> TempFiles {
    if let Some(temps) = cx.try_global::<SaveTemps>() {
        return temps.0.clone();
    }
    let temps = TempFiles::default();
    cx.set_global(SaveTemps(temps.clone()));
    let on_quit = temps.clone();
    cx.on_app_quit(move |_| {
        let removed = on_quit.remove_all();
        if removed > 0 {
            tracing::info!(removed, "removed the temp files of unfinished saves");
        }
        async {}
    })
    .detach();
    temps
}

/// Holds back the writes [`hold_saves_for_test`] asked for.
struct SaveHold(Shared<oneshot::Receiver<()>>);

impl Global for SaveHold {}

/// Makes every write started from now on wait until the returned sender is
/// dropped, as a slow disk would.
#[doc(hidden)]
pub fn hold_saves_for_test(cx: &mut App) -> oneshot::Sender<()> {
    let (tx, rx) = oneshot::channel();
    cx.set_global(SaveHold(rx.shared()));
    tx
}

/// What a write waits for before it starts: only set by
/// [`hold_saves_for_test`].
pub(crate) fn save_hold(cx: &App) -> Option<Shared<oneshot::Receiver<()>>> {
    cx.try_global::<SaveHold>().map(|hold| hold.0.clone())
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

/// Where the Save As dialog for the document at `path` starts, and the
/// name it suggests (spec §4.3, §4.4): for an Untitled document, the
/// opened folder (in single-file mode, the file's folder) and its tab
/// title without the characters a file name cannot hold, else
/// `Untitled-N`; for a file, its folder and its name. No extension is
/// added.
pub fn save_dialog_start(wb: &Workbench, path: &Path, cx: &App) -> (PathBuf, String) {
    if crate::untitled::is_untitled(path) {
        let folder = wb
            .is_single_file()
            .then(|| wb.opened_paths().pop())
            .flatten()
            .and_then(|file| file.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| wb.root().to_path_buf());
        let title = crate::editor_groups::untitled_title(Some(wb.documents().read(cx)), path);
        let name = crate::untitled::suggested_file_name(&title)
            .unwrap_or_else(|| crate::untitled::name(path));
        return (folder, name);
    }
    let path = wb.absolute_path(path);
    let folder = path
        .parent()
        .map_or_else(|| wb.root().to_path_buf(), Path::to_path_buf);
    (folder, display_name(&path))
}

/// Whether the absolute paths `a` and `b` name one file: equal, or equal
/// once symlinks are resolved (the save dialog may answer with the
/// resolved form of the folder it was given).
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (fs::canonicalize(a), fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// Saves the document of the active code view or preview; an Untitled one
/// goes through Save As.
fn save_active(
    wb: &mut Workbench,
    opts: SaveOptions,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(path) = active_document_path(wb, cx) else {
        return;
    };
    if crate::untitled::is_untitled(&path) {
        wb.save_as(path, window, cx).detach();
        return;
    }
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
/// asking and never touching a conflicted or deleted one, or one with
/// invalid bytes the user has not agreed to replace (spec §6.4).
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
        self.settings()
            .read(cx)
            .get::<String>("files.autoSave", language.as_deref(), cx)
            .unwrap_or_else(|| "off".into())
    }

    fn auto_save_delay(&self, path: &Path, cx: &App) -> Duration {
        let language = self
            .documents()
            .read(cx)
            .language_id_of(path)
            .map(|id| id.to_string());
        let ms = self
            .settings()
            .read(cx)
            .get::<u64>("files.autoSaveDelay", language.as_deref(), cx)
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
            // Keep Mine ends a conflict. The timer that ran during the
            // conflict skipped the save and is gone, so arm it again.
            DocumentEvent::ConflictChanged {
                path,
                conflict: false,
            } if documents.read(cx).is_dirty(path)
                && self.auto_save_mode(path, cx) == "afterDelay" =>
            {
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

    /// The dirty documents `files.autoSave` saves (not `"off"`) that can
    /// be saved without asking: neither in conflict with the disk, nor
    /// deleted, nor holding invalid bytes the user has not agreed to
    /// replace. Saved as the app quits with hot exit on.
    pub(crate) fn auto_save_paths(&self, cx: &App) -> Vec<Arc<Path>> {
        let docs = self.documents().read(cx);
        docs.dirty_paths()
            .into_iter()
            .filter(|path| {
                !docs.has_conflict(path)
                    && !docs.needs_lossy_consent(path)
                    && docs.state(path) == Some(&DocState::Ready)
                    && self.auto_save_mode(path, cx) != "off"
            })
            .collect()
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
            overwrite: false,
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
                overwrite: false,
            };
            save_active(wb, opts, window, cx)
        },
    );
    commands.register(
        "workbench.action.files.saveAs",
        "File: Save As...",
        |wb, _, window, cx| {
            // Only a code view: Search, diffs and previews have no file of
            // their own to save elsewhere.
            let Some(view) = wb.active_code_view(cx) else {
                return;
            };
            let path = view.read(cx).path().clone();
            wb.save_as(path, window, cx).detach();
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
                overwrite: false,
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

    fn apply_with(text: &str, f: Participant, rows: &[u32]) -> String {
        let buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
        let snapshot = buffer.snapshot();
        let mut out = text.to_string();
        for (range, new) in f(&snapshot, rows).into_iter().rev() {
            out.replace_range(range, &new);
        }
        out
    }

    #[test]
    fn final_newlines_follow_vs_code() {
        assert_eq!(apply("a\n\n\n", trim_final_newlines), "a\n");
        assert_eq!(apply("a\n", trim_final_newlines), "a\n");
        assert_eq!(apply("a", trim_final_newlines), "a");
        assert_eq!(apply("", trim_final_newlines), "");
        assert_eq!(apply("\n\n\n", trim_final_newlines), "");
        assert_eq!(apply("\n\n", trim_final_newlines), "");
        assert_eq!(apply("\n", trim_final_newlines), "");
        assert_eq!(apply("a\n  \n\n", trim_final_newlines), "a\n  \n");
        // Autosave: lines up to the furthest cursor stay.
        assert_eq!(apply_with("a\n\n\n", trim_final_newlines, &[2]), "a\n\n\n");
        assert_eq!(
            apply_with("a\n\n\n\n", trim_final_newlines, &[2]),
            "a\n\n\n"
        );
        assert_eq!(apply_with("\n\n\n", trim_final_newlines, &[0]), "\n");
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
