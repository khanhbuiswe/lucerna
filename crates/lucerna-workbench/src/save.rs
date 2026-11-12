//! Saving (spec §6.1): the save options, the save participants, the write
//! itself (off the main thread) and the save and revert commands.
//!
//! The flow lives in [`crate::Documents::save`]: it asks about a conflict with the
//! disk, runs the participants as one undo step, captures the text and
//! writes it here in the background, then marks what it captured as saved.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use encoding_rs::Encoding;
use gpui::{Context, Window};
use lucerna_text::{Buffer, BufferSnapshot, LineEnding, Point, WriteError, Written, write_bytes};

use crate::documents::Edits;
use crate::{CommandRegistry, Workbench};

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

/// A save participant: the edits it makes to `text`.
pub(crate) type Participant = fn(&BufferSnapshot) -> Edits;

/// The participants enabled for a document of `language`, in the order
/// they run.
pub(crate) fn participants(
    settings: &lucerna_settings::SettingsStore,
    language: Option<&str>,
) -> Vec<Participant> {
    let on = |key: &str| settings.get::<bool>(key, language).unwrap_or(false);
    let mut out: Vec<Participant> = Vec::new();
    if on("files.trimTrailingWhitespace") {
        out.push(trim_trailing_whitespace);
    }
    // `editor.formatOnSave` runs here from step 4.6.
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
fn trim_trailing_whitespace(text: &BufferSnapshot) -> Edits {
    let mut edits = Vec::new();
    for row in 0..text.line_count() {
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
fn insert_final_newline(text: &BufferSnapshot) -> Edits {
    let len = text.len_bytes();
    if len == 0 || text.byte_at(len - 1) == b'\n' {
        return Vec::new();
    }
    vec![(len..len, Arc::from("\n"))]
}

/// Collapses the line breaks at the end of the text to one.
fn trim_final_newlines(text: &BufferSnapshot) -> Edits {
    let len = text.len_bytes();
    let mut start = len;
    while start > 0 && text.byte_at(start - 1) == b'\n' {
        start -= 1;
    }
    if len - start > 1 {
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
}

impl WriteJob {
    /// Encodes and writes the text to `path`; also returns the file's
    /// metadata after the write.
    pub(crate) fn run(self, path: &Path) -> Result<(Written, Option<fs::Metadata>), WriteError> {
        let bytes = Buffer::encode(
            &self.snapshot,
            self.encoding,
            self.has_bom,
            self.line_ending,
        )?;
        let written = write_bytes(path, &bytes)?;
        Ok((written, fs::metadata(path).ok()))
    }
}

/// The message for a failed save of `path`.
pub(crate) fn failure_message(path: &Path, err: &WriteError) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    format!("Failed to save '{name}': {err}")
}

/// Saves the document of the active code view.
fn save_active(
    wb: &mut Workbench,
    opts: SaveOptions,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(view) = wb.active_code_view(cx) else {
        return;
    };
    let path = view.read(cx).path().clone();
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
    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: true,
    };
    cx.spawn_in(window, async move |_, cx| {
        for path in paths {
            let Ok(save) = cx
                .update(|window, cx| documents.update(cx, |d, cx| d.save(&path, opts, window, cx)))
            else {
                return;
            };
            // A failure is reported in the status bar; the rest still save.
            save.await.ok();
        }
    })
    .detach();
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
            let Some(view) = wb.active_code_view(cx) else {
                return;
            };
            let path = view.read(cx).path().clone();
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
        for (range, new) in f(&snapshot).into_iter().rev() {
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
}
