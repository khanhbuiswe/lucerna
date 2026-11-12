//! Saving through the workbench: the save commands and their keys, the
//! save participants, edits made while a save writes, a failed save, the
//! conflict prompt, the dirty and conflict dots on tabs, revert, and how
//! saves and reloads from disk interleave.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, DocState, DocumentEvent, Documents, OpenMode, SaveOptions, SaveReason, Workbench,
    status_message,
};
use tempfile::TempDir;

const CONFLICT_MESSAGE: &str =
    "This file has changed on disk since you started editing it. Do you want to overwrite it?";

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings,
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn documents(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    })
}

fn open(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(name)), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = active_view(wb, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    settle(cx);
    view
}

/// A workbench on a temporary folder holding `file.txt` with `text`, the
/// file open and its view focused.
fn editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    let view = open(&wb, "file.txt", cx);
    (dir, wb, view, cx)
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn input(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    settle(cx);
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    settle(cx);
}

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

fn disk(dir: &TempDir, name: &str) -> String {
    fs::read_to_string(dir.path().join(name)).unwrap()
}

fn is_dirty(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.is_dirty(Path::new(name)))
}

fn has_conflict(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.has_conflict(Path::new(name)))
}

fn changed(wb: &Entity<Workbench>, name: &str, kind: ChangeKind, cx: &mut VisualTestContext) {
    let change = Change {
        path: Arc::from(Path::new(name)),
        kind,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
}

/// Another program writes `text` to `file.txt`, and the watcher reports it.
fn external_write(wb: &Entity<Workbench>, dir: &TempDir, text: &str, cx: &mut VisualTestContext) {
    fs::write(dir.path().join("file.txt"), text).unwrap();
    changed(wb, "file.txt", ChangeKind::Modified, cx);
}

fn drawn(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    draw(cx);
    cx.debug_bounds(selector).is_some()
}

fn message(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| status_message::current(cx).map(|m| m.to_string()))
}

/// Every event `Documents` emits from now on.
fn record_events(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> (Rc<RefCell<Vec<DocumentEvent>>>, gpui::Subscription) {
    let docs = documents(wb, cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    let sub = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&docs, move |_, ev: &DocumentEvent, _| {
            events.borrow_mut().push(ev.clone())
        })
    });
    (events, sub)
}

fn edited_count(events: &Rc<RefCell<Vec<DocumentEvent>>>) -> usize {
    events
        .borrow()
        .iter()
        .filter(|e| matches!(e, DocumentEvent::Edited { .. }))
        .count()
}

/// The user's unsaved `x` against a disk that changed to `disk\n`.
fn conflicted(
    cx: &mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &mut VisualTestContext,
) {
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "disk\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    assert_eq!(text_of(&view, cx), "xabc\n");
    (dir, wb, view, cx)
}

#[gpui::test]
fn save_writes_bytes_and_cleans(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    assert!(is_dirty(&wb, "file.txt", cx));
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!drawn("tab-dirty-0-0", cx));
}

#[gpui::test]
fn save_writes_even_when_clean(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, _wb, _view, cx) = editor("abc\n", cx);
    fs::remove_file(dir.path().join("file.txt")).unwrap();
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
}

#[gpui::test]
fn save_keeps_encoding_bom_and_line_endings(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = TempDir::new().unwrap();
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "a\r\nb\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(dir.path().join("file.txt"), &bytes).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    open(&wb, "file.txt", cx);
    keys("cmd-s", cx);
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), bytes);
    input("x", cx);
    keys("cmd-s", cx);
    let mut expected = vec![0xFF, 0xFE];
    for unit in "xa\r\nb\r\n".encode_utf16() {
        expected.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(fs::read(dir.path().join("file.txt")).unwrap(), expected);
}

#[gpui::test]
fn save_participants_follow_settings(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.trimTrailingWhitespace": true, "files.insertFinalNewline": true}"#),
    );
    let (dir, wb, view, cx) = editor("a  \nb", cx);
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "a\nb\n");
    assert_eq!(text_of(&view, cx), "a\nb\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "a  \nb", "one undo step");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn trim_final_newlines_collapses_to_one(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.trimFinalNewlines": true}"#));
    let (dir, _wb, _view, cx) = editor("a\n\n\n", cx);
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "a\n");
}

#[gpui::test]
fn participants_run_in_order(cx: &mut TestAppContext) {
    // Trimming whitespace first leaves the blank lines empty, so trimming
    // final newlines removes them.
    init(
        cx,
        Some(
            r#"{"files.trimTrailingWhitespace": true, "files.insertFinalNewline": true, "files.trimFinalNewlines": true}"#,
        ),
    );
    let (dir, _wb, _view, cx) = editor("a \n  \n\t\n", cx);
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "a\n");
}

#[gpui::test]
fn save_without_formatting_skips_participants(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.trimTrailingWhitespace": true, "files.insertFinalNewline": true}"#),
    );
    let (dir, wb, view, cx) = editor("a  \nb", cx);
    input("x", cx);
    keys("cmd-k s", cx);
    assert_eq!(disk(&dir, "file.txt"), "xa  \nb");
    assert_eq!(text_of(&view, cx), "xa  \nb");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn save_all_saves_dirty_documents_only(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.trimTrailingWhitespace": true}"#));
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    fs::write(dir.path().join("b.txt"), "b  \n").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    open(&wb, "b.txt", cx);
    open(&wb, "a.txt", cx);
    input("x", cx);
    keys("alt-cmd-s", cx);
    assert_eq!(disk(&dir, "a.txt"), "xa\n");
    assert!(!is_dirty(&wb, "a.txt", cx));
    assert_eq!(
        disk(&dir, "b.txt"),
        "b  \n",
        "a clean document is not saved"
    );
}

#[gpui::test]
fn own_write_does_not_reload_or_conflict(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    keys("cmd-s", cx);
    let (events, _sub) = record_events(&wb, cx);
    changed(&wb, "file.txt", ChangeKind::Modified, cx);
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert_eq!(edited_count(&events), 0);
    assert_eq!(text_of(&view, cx), "xabc\n");
}

#[gpui::test]
fn save_emits_saved_and_dirty_changed(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    let (events, _sub) = record_events(&wb, cx);
    keys("cmd-s", cx);
    let events = events.borrow();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DocumentEvent::Saved { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DocumentEvent::DirtyChanged { dirty: false, .. }))
    );
}

#[gpui::test]
fn edit_during_save_stays_dirty(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    let docs = documents(&wb, cx);
    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: true,
    };
    let save = cx.update(|window, cx| {
        docs.update(cx, |d, cx| d.save(Path::new("file.txt"), opts, window, cx))
    });
    // The write runs on a background thread, which has not run yet.
    let id = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("file.txt"),
            vec![(0..0, Arc::from("y"))],
            None,
            None,
            id,
            cx,
        )
    })
    .expect("loaded");
    settle(cx);
    let saved = futures::FutureExt::now_or_never(save);
    assert_eq!(saved, Some(Ok(true)), "the save finished");
    assert_eq!(disk(&dir, "file.txt"), "xabc\n", "the captured text");
    assert_eq!(text_of(&view, cx), "yxabc\n");
    assert!(is_dirty(&wb, "file.txt", cx), "the later edit is unsaved");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx), "back to what was saved");
}

#[cfg(unix)]
#[gpui::test]
fn failed_save_keeps_dirty_and_reports(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;

    init(cx, None);
    let dir = TempDir::new().unwrap();
    let ro = dir.path().join("ro");
    fs::create_dir(&ro).unwrap();
    fs::write(ro.join("f.txt"), "abc\n").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    open(&wb, "ro/f.txt", cx);
    input("x", cx);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    keys("cmd-s", cx);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(is_dirty(&wb, "ro/f.txt", cx));
    assert_eq!(fs::read_to_string(ro.join("f.txt")).unwrap(), "abc\n");
    let message = message(cx).expect("a status message");
    assert!(message.starts_with("Failed to save 'f.txt': "), "{message}");
    assert!(drawn("status-message", cx));
    assert_eq!(fs::read_dir(&ro).unwrap().count(), 1, "no temp file left");
}

#[gpui::test]
fn status_message_clears_after_four_seconds(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, _view, cx) = editor("abc\n", cx);
    cx.update(|_, cx| status_message::show("first", cx));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.update(|_, cx| status_message::show("second", cx));
    cx.executor().advance_clock(Duration::from_secs(3));
    assert_eq!(
        message(cx).as_deref(),
        Some("second"),
        "replaced, not cut short"
    );
    assert!(drawn("status-message", cx));
    cx.executor().advance_clock(Duration::from_secs(1));
    assert_eq!(message(cx), None);
    assert!(!drawn("status-message", cx));
}

#[gpui::test]
fn conflict_prompt_overwrite(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = conflicted(cx);
    keys("cmd-s", cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((CONFLICT_MESSAGE.to_string(), String::new()))
    );
    assert_eq!(disk(&dir, "file.txt"), "disk\n", "nothing written yet");
    cx.simulate_prompt_answer("Overwrite");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn conflict_prompt_reload(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = conflicted(cx);
    keys("cmd-s", cx);
    cx.simulate_prompt_answer("Reload");
    settle(cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert_eq!(disk(&dir, "file.txt"), "disk\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\n", "the user's text comes back");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn conflict_prompt_cancel(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = conflicted(cx);
    keys("cmd-s", cx);
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "disk\n");
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
    assert!(has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn overwrite_setting_skips_prompt(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.saveConflictResolution": "overwriteFileOnDisk"}"#),
    );
    let (dir, wb, _view, cx) = conflicted(cx);
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn tab_dot_states(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    assert!(!drawn("tab-dirty-0-0", cx));
    assert!(!drawn("tab-conflict-0-0", cx));
    assert!(drawn("tab-close-0-0", cx), "a clean active tab shows its ×");

    input("x", cx);
    assert!(drawn("tab-dirty-0-0", cx));
    assert!(!drawn("tab-close-0-0", cx), "the dot takes the ×'s place");

    external_write(&wb, &dir, "disk\n", cx);
    assert!(drawn("tab-conflict-0-0", cx));
    assert!(!drawn("tab-dirty-0-0", cx));
}

#[gpui::test]
fn revert_reloads_from_disk_undoably(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    run(&wb, "workbench.action.files.revert", cx);
    assert_eq!(text_of(&view, cx), "abc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\n");
}

#[gpui::test]
fn revert_clears_the_conflict(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = conflicted(cx);
    run(&wb, "workbench.action.files.revert", cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn saving_a_deleted_file_recreates_it(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::remove_file(dir.path().join("file.txt")).unwrap();
    changed(&wb, "file.txt", ChangeKind::Removed, cx);
    let docs = documents(&wb, cx);
    let state = |cx: &mut VisualTestContext| {
        docs.read_with(cx, |d, _| d.state(Path::new("file.txt")).cloned())
    };
    assert_eq!(state(cx), Some(DocState::Deleted));
    let (events, _sub) = record_events(&wb, cx);
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert_eq!(state(cx), Some(DocState::Ready));
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!drawn("tab-deleted-0-0", cx));
    // Language servers that let the deleted file go take it up again.
    assert_eq!(edited_count(&events), 1);
}

/// Final review, finding 1: undoing a conflicted document back to clean
/// keeps the user's step redoable. The conflict stays; the disk text is
/// taken on the next disk event, on Revert, or when the text is clean with
/// nothing left to redo.
#[gpui::test]
fn undo_to_clean_while_conflicted_keeps_redo(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = conflicted(cx);
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "abc\n");
    assert!(has_conflict(&wb, "file.txt", cx));
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(drawn("tab-conflict-0-0", cx));
    keys("cmd-shift-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(has_conflict(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
    // Undone again, the next disk event takes the disk text.
    keys("cmd-z", cx);
    external_write(&wb, &dir, "disk 2\n", cx);
    assert_eq!(text_of(&view, cx), "disk 2\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

/// Undone to clean while conflicted, Revert takes the disk text.
#[gpui::test]
fn undo_to_clean_while_conflicted_then_revert(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = conflicted(cx);
    keys("cmd-z", cx);
    run(&wb, "workbench.action.files.revert", cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

/// Redone back to the saved text with nothing left to redo: no user text
/// is lost, so the disk text is taken.
#[gpui::test]
fn redo_to_clean_while_conflicted_reloads(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    keys("cmd-s", cx);
    keys("cmd-z", cx);
    assert!(is_dirty(&wb, "file.txt", cx));
    external_write(&wb, &dir, "disk\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    keys("cmd-shift-z", cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

/// Controller ruling (c): a deleted file that comes back while the text is
/// dirty is announced to listeners, as one that comes back clean is.
#[gpui::test]
fn deleted_file_returning_while_dirty_is_announced(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::remove_file(dir.path().join("file.txt")).unwrap();
    changed(&wb, "file.txt", ChangeKind::Removed, cx);
    let (events, _sub) = record_events(&wb, cx);
    fs::write(dir.path().join("file.txt"), "new\n").unwrap();
    changed(&wb, "file.txt", ChangeKind::Added, cx);
    let docs = documents(&wb, cx);
    let state = docs.read_with(cx, |d, _| d.state(Path::new("file.txt")).cloned());
    assert_eq!(state, Some(DocState::Ready));
    assert!(has_conflict(&wb, "file.txt", cx));
    assert_eq!(text_of(&view, cx), "xabc\n");
    let events = events.borrow();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DocumentEvent::Edited { patch, .. } if patch.is_empty())),
        "{events:?}"
    );
}

/// Starts a save of `file.txt` without waiting for it.
fn start_save(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> gpui::Task<SaveResult> {
    let docs = documents(wb, cx);
    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: false,
    };
    cx.update(|window, cx| docs.update(cx, |d, cx| d.save(Path::new("file.txt"), opts, window, cx)))
}

type SaveResult = Result<bool, lucerna_text::WriteError>;

/// Delivers the watcher's report for `file.txt` without letting the
/// reload it starts run.
fn modified_unsettled(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
}

/// Writes `text` to `file.txt` the first time the document is saved, as an
/// agent writing right after our write landed.
fn write_after_first_save(
    wb: &Entity<Workbench>,
    dir: &TempDir,
    text: &'static str,
    cx: &mut VisualTestContext,
) -> gpui::Subscription {
    let docs = documents(wb, cx);
    let file = dir.path().join("file.txt");
    let fired = Rc::new(std::cell::Cell::new(false));
    cx.update(|_, cx| {
        cx.subscribe(&docs, move |_, ev: &DocumentEvent, _| {
            if matches!(ev, DocumentEvent::Saved { .. }) && !fired.replace(true) {
                fs::write(&file, text).unwrap();
            }
        })
    })
}

/// An external write that lands before the watcher reports it is never
/// written over: whatever order the reload and the save run in, the save
/// ends in the conflict prompt.
#[gpui::test(iterations = 30)]
fn external_write_racing_a_save_ends_in_conflict(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::write(dir.path().join("file.txt"), "ext\n").unwrap();
    modified_unsettled(&wb, cx);
    start_save(&wb, cx).detach();
    settle(cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((CONFLICT_MESSAGE.to_string(), String::new()))
    );
    assert_eq!(disk(&dir, "file.txt"), "ext\n");
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(has_conflict(&wb, "file.txt", cx));
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "ext\n");
}

/// The same race under a clean text: the file's text is taken, not
/// written over, and nothing asks.
#[gpui::test(iterations = 30)]
fn external_write_racing_a_clean_save_is_taken(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    fs::write(dir.path().join("file.txt"), "ext\n").unwrap();
    modified_unsettled(&wb, cx);
    start_save(&wb, cx).detach();
    settle(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "ext\n");
    assert_eq!(text_of(&view, cx), "ext\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
}

/// The disk is checked right before writing, even with no watcher report.
#[gpui::test]
fn save_checks_the_file_before_writing(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::write(dir.path().join("file.txt"), "agent wrote this\n").unwrap();
    keys("cmd-s", cx);
    assert!(cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "agent wrote this\n");
    cx.simulate_prompt_answer("Overwrite");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn overwrite_setting_skips_the_disk_check(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.saveConflictResolution": "overwriteFileOnDisk"}"#),
    );
    let (dir, _wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::write(dir.path().join("file.txt"), "agent wrote this\n").unwrap();
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
}

/// ⌘S twice: an agent writes between the two writes. The second save
/// checks the disk when its turn comes and asks instead of writing over.
#[gpui::test]
fn second_save_does_not_write_over_an_agent(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    let _sub = write_after_first_save(&wb, &dir, "agent\n", cx);
    start_save(&wb, cx).detach();
    // Typed while the first save writes, then saved again.
    let docs = documents(&wb, cx);
    let id = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("file.txt"),
            vec![(0..0, Arc::from("y"))],
            None,
            None,
            id,
            cx,
        )
    })
    .expect("loaded");
    start_save(&wb, cx).detach();
    settle(cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((CONFLICT_MESSAGE.to_string(), String::new()))
    );
    assert_eq!(disk(&dir, "file.txt"), "agent\n");
    assert!(has_conflict(&wb, "file.txt", cx));
    cx.simulate_prompt_answer("Overwrite");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "yxabc\n");
}

/// A save queued behind a revert looks at the conflict when its turn
/// comes: the revert ended it, so the save writes without asking.
#[gpui::test]
fn save_queued_behind_a_revert_rechecks_the_conflict(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = conflicted(cx);
    let docs = documents(&wb, cx);
    docs.update(cx, |d, cx| d.revert(Path::new("file.txt"), cx))
        .detach();
    start_save(&wb, cx).detach();
    settle(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(text_of(&view, cx), "disk\n");
    assert_eq!(disk(&dir, "file.txt"), "disk\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
}

/// A reload asked for while a write is in flight waits for it, then runs.
#[gpui::test]
fn reload_deferred_by_a_write_runs_after_it(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    let _sub = write_after_first_save(&wb, &dir, "ext\n", cx);
    start_save(&wb, cx).detach();
    modified_unsettled(&wb, cx);
    settle(cx);
    assert_eq!(text_of(&view, cx), "ext\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

/// A save dropped before its write ended does not hold a reload back.
#[gpui::test]
fn dropped_save_does_not_hold_reloads_back(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    let save = start_save(&wb, cx);
    fs::write(dir.path().join("file.txt"), "ext\n").unwrap();
    modified_unsettled(&wb, cx);
    drop(save);
    settle(cx);
    settle(cx);
    assert_eq!(text_of(&view, cx), "ext\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn eol_setting_applies_to_files_without_line_breaks(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.eol": "\r\n"}"#));
    let (dir, _wb, _view, cx) = editor("ab", cx);
    keys("enter", cx);
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "\r\nab");
}

#[gpui::test]
fn eol_setting_leaves_files_with_line_breaks(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.eol": "\r\n"}"#));
    let (dir, _wb, _view, cx) = editor("a\nb", cx);
    keys("enter", cx);
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "\na\nb");
}

const CLOSE_DETAIL: &str = "Your changes will be lost if you don't save them.";

fn close_message(name: &str) -> String {
    format!("Do you want to save the changes you made to {name}?")
}

/// The paths and pinned flags of every tab, group by group.
fn tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<Vec<(String, bool)>> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .map(|g| g.tabs.iter().map(|t| (t.title(), t.pinned)).collect())
            .collect()
    })
}

/// `file.txt` with the unsaved `x`, ready to be closed.
fn dirty_editor(cx: &mut TestAppContext) -> (TempDir, Entity<Workbench>, &mut VisualTestContext) {
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    assert!(is_dirty(&wb, "file.txt", cx));
    // Only the tab holds the view, so closing it closes the document.
    drop(view);
    (dir, wb, cx)
}

fn close_active(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    run(wb, "workbench.action.closeActiveEditor", cx);
}

#[gpui::test]
fn close_dirty_tab_prompts_save(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = dirty_editor(cx);
    close_active(&wb, cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((close_message("file.txt"), CLOSE_DETAIL.to_string()))
    );
    assert_eq!(tabs(&wb, cx).concat().len(), 1, "open until answered");
    cx.simulate_prompt_answer("Save");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(tabs(&wb, cx).concat().is_empty());
}

#[gpui::test]
fn close_dirty_tab_dont_save_discards(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, old_view, cx) = editor("abc\n", cx);
    input("x", cx);
    // A view held elsewhere does not keep the document (and its history)
    // alive.
    close_active(&wb, cx);
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    assert!(tabs(&wb, cx).concat().is_empty());
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    let docs = documents(&wb, cx);
    assert!(
        docs.read_with(cx, |d, _| d.open_paths().is_empty()),
        "the document is dropped"
    );
    let view = open(&wb, "file.txt", cx);
    assert_eq!(text_of(&view, cx), "abc\n", "the next open reads the disk");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "abc\n", "with no history");
    assert!(!is_dirty(&wb, "file.txt", cx));
    drop(old_view);
}

#[gpui::test]
fn close_dirty_tab_cancel_keeps(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = dirty_editor(cx);
    close_active(&wb, cx);
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(tabs(&wb, cx).concat().len(), 1);
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
    assert_eq!(text_of(&active_view(&wb, cx), cx), "xabc\n");
}

#[gpui::test]
fn close_one_of_two_tabs_on_same_file_does_not_prompt(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = dirty_editor(cx);
    run(&wb, "workbench.action.splitEditor", cx);
    assert_eq!(tabs(&wb, cx).concat().len(), 2);
    close_active(&wb, cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(tabs(&wb, cx).concat().len(), 1);
    assert!(is_dirty(&wb, "file.txt", cx));

    close_active(&wb, cx);
    assert!(cx.has_pending_prompt(), "the last tab of the file asks");
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
}

/// `file.txt` and `other.txt`, both with unsaved changes.
fn two_dirty(cx: &mut TestAppContext) -> (TempDir, Entity<Workbench>, &mut VisualTestContext) {
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::write(dir.path().join("other.txt"), "def\n").unwrap();
    open(&wb, "other.txt", cx);
    input("y", cx);
    (dir, wb, cx)
}

fn confirm_quit(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> gpui::Task<bool> {
    let quit = cx.update(|window, cx| wb.update(cx, |wb, cx| wb.confirm_quit(window, cx)));
    settle(cx);
    quit
}

fn quit_answer(quit: gpui::Task<bool>) -> bool {
    use futures::FutureExt as _;
    quit.now_or_never().expect("the quit prompt was answered")
}

#[gpui::test]
fn quit_with_dirty_prompts_and_cancel_aborts(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = two_dirty(cx);
    let quit = confirm_quit(&wb, cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((
            "Do you want to save the changes to the following 2 files?".to_string(),
            format!("file.txt\nother.txt\n\n{CLOSE_DETAIL}"),
        ))
    );
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(!quit_answer(quit));
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "other.txt", cx));
}

#[gpui::test]
fn quit_save_all_saves_every_file(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = two_dirty(cx);
    let quit = confirm_quit(&wb, cx);
    cx.simulate_prompt_answer("Save All");
    settle(cx);
    assert!(quit_answer(quit));
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert_eq!(disk(&dir, "other.txt"), "ydef\n");
}

#[gpui::test]
fn quit_dont_save_quits_without_saving(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = two_dirty(cx);
    let quit = confirm_quit(&wb, cx);
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    assert!(quit_answer(quit));
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
}

#[gpui::test]
fn quit_with_one_dirty_file_uses_the_close_wording(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = dirty_editor(cx);
    let quit = confirm_quit(&wb, cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((close_message("file.txt"), CLOSE_DETAIL.to_string()))
    );
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(!quit_answer(quit));
}

#[gpui::test]
fn quit_with_nothing_dirty_does_not_ask(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, _view, cx) = editor("abc\n", cx);
    let quit = confirm_quit(&wb, cx);
    assert!(!cx.has_pending_prompt());
    assert!(quit_answer(quit));
}

#[gpui::test]
fn window_close_with_dirty_prompts(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = two_dirty(cx);
    assert!(!cx.simulate_close(), "the window waits for the answer");
    settle(cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(cx.windows().len(), 1, "Cancel keeps the window");
    assert!(is_dirty(&wb, "file.txt", cx));

    assert!(!cx.simulate_close());
    settle(cx);
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(cx.windows().is_empty(), "Don't Save closes the window");
}

#[gpui::test]
fn editing_a_preview_pins_it(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), "abc\n").unwrap();
    fs::write(dir.path().join("other.txt"), "def\n").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.txt")), OpenMode::Preview, cx)
    });
    settle(cx);
    assert_eq!(tabs(&wb, cx), vec![vec![("file.txt".to_string(), false)]]);
    let view = active_view(&wb, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    input("x", cx);
    assert_eq!(
        tabs(&wb, cx),
        vec![vec![("file.txt".to_string(), true)]],
        "a dirty file is never a preview (VS Code)"
    );
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("other.txt")), OpenMode::Preview, cx)
    });
    settle(cx);
    assert_eq!(
        tabs(&wb, cx),
        vec![vec![
            ("file.txt".to_string(), true),
            ("other.txt".to_string(), false)
        ]]
    );
}

/// Types `y` at the start of `file.txt` right after its next save
/// lands, before whoever waits on the save goes on.
fn edit_after_next_save(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> gpui::Subscription {
    let docs = documents(wb, cx);
    let id = view.entity_id();
    let fired = Rc::new(std::cell::Cell::new(false));
    cx.update(|_, cx| {
        cx.subscribe(&docs, move |docs, ev: &DocumentEvent, cx| {
            if matches!(ev, DocumentEvent::Saved { .. }) && !fired.replace(true) {
                cx.defer(move |cx| {
                    docs.update(cx, |d, cx| {
                        d.apply_edits(
                            Path::new("file.txt"),
                            vec![(0..0, Arc::from("y"))],
                            None,
                            None,
                            id,
                            cx,
                        )
                    })
                    .expect("loaded");
                });
            }
        })
    })
}

#[gpui::test]
fn close_save_keeps_tab_when_edited_during_the_save(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    let _edit = edit_after_next_save(&wb, &view, cx);
    close_active(&wb, cx);
    cx.simulate_prompt_answer("Save");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert_eq!(text_of(&view, cx), "yxabc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
    assert_eq!(
        tabs(&wb, cx).concat().len(),
        1,
        "the later edit keeps the tab"
    );
}

#[gpui::test]
fn quit_save_all_aborts_when_edited_during_the_save(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    let _edit = edit_after_next_save(&wb, &view, cx);
    let quit = confirm_quit(&wb, cx);
    cx.simulate_prompt_answer("Save");
    settle(cx);
    assert!(!quit_answer(quit), "the later edit stops the quit");
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn close_save_cancelled_at_the_conflict_keeps_tab(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = conflicted(cx);
    close_active(&wb, cx);
    cx.simulate_prompt_answer("Save");
    settle(cx);
    assert_eq!(
        cx.pending_prompt(),
        Some((CONFLICT_MESSAGE.to_string(), String::new()))
    );
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(tabs(&wb, cx).concat().len(), 1);
    assert_eq!(disk(&dir, "file.txt"), "disk\n");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn quit_save_all_cancelled_at_the_conflict_aborts(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = conflicted(cx);
    let quit = confirm_quit(&wb, cx);
    cx.simulate_prompt_answer("Save");
    settle(cx);
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(!quit_answer(quit));
    assert_eq!(disk(&dir, "file.txt"), "disk\n");
}

#[cfg(unix)]
#[gpui::test]
fn close_save_that_fails_keeps_tab(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;

    init(cx, None);
    let dir = TempDir::new().unwrap();
    let ro = dir.path().join("ro");
    fs::create_dir(&ro).unwrap();
    fs::write(ro.join("f.txt"), "abc\n").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    open(&wb, "ro/f.txt", cx);
    input("x", cx);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    close_active(&wb, cx);
    cx.simulate_prompt_answer("Save");
    settle(cx);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(tabs(&wb, cx).concat().len(), 1);
    assert!(is_dirty(&wb, "ro/f.txt", cx));
    assert!(message(cx).is_some_and(|m| m.starts_with("Failed to save 'f.txt': ")));
}

const AFTER_DELAY: &str = r#"{"files.autoSave": "afterDelay"}"#;

fn advance(ms: u64, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

/// Makes the test window the active one, as in a running app.
fn activate(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    settle(cx);
}

/// Moves focus out of the code view, to Quick Open's input.
fn blur(cx: &mut VisualTestContext) {
    keys("cmd-p", cx);
}

#[gpui::test]
fn auto_save_is_off_by_default(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    advance(5000, cx);
    blur(cx);
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn after_delay_saves_after_1000ms_of_quiet(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    advance(999, cx);
    assert_eq!(disk(&dir, "file.txt"), "abc\n", "not yet");
    advance(1, cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn after_delay_restarts_on_each_edit(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, _wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    advance(600, cx);
    input("y", cx);
    advance(600, cx);
    assert_eq!(
        disk(&dir, "file.txt"),
        "abc\n",
        "restarted by the second edit"
    );
    advance(400, cx);
    assert_eq!(disk(&dir, "file.txt"), "xyabc\n");
}

#[gpui::test]
fn after_delay_follows_the_delay_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.autoSave": "afterDelay", "files.autoSaveDelay": 200}"#),
    );
    let (dir, _wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    advance(199, cx);
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
    advance(1, cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
}

#[gpui::test]
fn after_delay_skips_conflicted(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, wb, _view, cx) = conflicted(cx);
    advance(1000, cx);
    assert_eq!(disk(&dir, "file.txt"), "disk\n");
    assert!(has_conflict(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
    assert!(cx.pending_prompt().is_none(), "an autosave never prompts");
}

#[gpui::test]
fn after_delay_skips_deleted(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::remove_file(dir.path().join("file.txt")).unwrap();
    changed(&wb, "file.txt", ChangeKind::Removed, cx);
    advance(1000, cx);
    assert!(!dir.path().join("file.txt").exists());
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn auto_save_conflict_found_while_writing_does_not_prompt(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    // Another program writes, and the watcher has not said so yet.
    fs::write(dir.path().join("file.txt"), "disk, longer\n").unwrap();
    advance(1000, cx);
    assert_eq!(disk(&dir, "file.txt"), "disk, longer\n");
    assert!(has_conflict(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
    assert!(cx.pending_prompt().is_none());
}

#[gpui::test]
fn undo_to_clean_cancels_the_timer(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    // Undo back to clean: nothing left to save.
    keys("cmd-z", cx);
    assert!(!is_dirty(&wb, "file.txt", cx));
    fs::write(dir.path().join("file.txt"), "other\n").unwrap();
    advance(1000, cx);
    assert_eq!(
        disk(&dir, "file.txt"),
        "other\n",
        "a clean document is not written"
    );
}

#[gpui::test]
fn on_focus_change_saves_when_focus_leaves(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.autoSave": "onFocusChange"}"#));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    activate(cx);
    input("x", cx);
    advance(5000, cx);
    assert_eq!(disk(&dir, "file.txt"), "abc\n", "no timer in this mode");
    blur(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn on_focus_change_ignores_focus_moving_to_the_find_widget(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.autoSave": "onFocusChange"}"#));
    let (dir, _wb, _view, cx) = editor("abc\n", cx);
    activate(cx);
    input("x", cx);
    keys("cmd-f", cx);
    assert_eq!(
        disk(&dir, "file.txt"),
        "abc\n",
        "the widget belongs to the view"
    );
}

#[gpui::test]
fn on_window_change_saves_when_the_window_loses_focus(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.autoSave": "onWindowChange"}"#));
    let (dir, _wb, _view, cx) = editor("abc\n", cx);
    activate(cx);
    input("x", cx);
    blur(cx);
    assert_eq!(
        disk(&dir, "file.txt"),
        "abc\n",
        "focus inside the window is not a window change"
    );
    // Activating another window deactivates this one.
    cx.update(|_, cx| {
        let other = cx
            .open_window(gpui::WindowOptions::default(), |_, cx| {
                gpui::AppContext::new(cx, |_| gpui::Empty)
            })
            .unwrap();
        other
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
    });
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
}

#[gpui::test]
fn auto_save_keeps_whitespace_on_cursor_lines(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.autoSave": "afterDelay", "files.trimTrailingWhitespace": true}"#),
    );
    let (dir, _wb, view, cx) = editor("a  \nb  \nc  \n", cx);
    // Cursor at the end of line 2, after the spaces; line 1 and 3 trim.
    view.update(cx, |v, cx| {
        v.select(
            lucerna_text::Point { row: 1, column: 3 }..lucerna_text::Point { row: 1, column: 3 },
            cx,
        )
    });
    input("x", cx);
    advance(1000, cx);
    assert_eq!(disk(&dir, "file.txt"), "a\nb  x\nc\n");
}

#[gpui::test]
fn explicit_save_trims_cursor_lines(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.trimTrailingWhitespace": true}"#));
    let (dir, _wb, view, cx) = editor("a  \nb  \n", cx);
    view.update(cx, |v, cx| {
        v.select(
            lucerna_text::Point { row: 1, column: 3 }..lucerna_text::Point { row: 1, column: 3 },
            cx,
        )
    });
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "a\nb\n");
}

#[gpui::test]
fn closing_the_tab_mid_delay_cancels_the_timer(cx: &mut TestAppContext) {
    init(cx, Some(AFTER_DELAY));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    advance(500, cx);
    run(&wb, "workbench.action.closeActiveEditor", cx);
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    advance(1000, cx);
    assert_eq!(disk(&dir, "file.txt"), "abc\n");
}

#[gpui::test]
fn auto_save_keeps_final_blank_lines_with_a_cursor(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.autoSave": "afterDelay", "files.trimFinalNewlines": true}"#),
    );
    let (dir, _wb, view, cx) = editor("a\n\n\n", cx);
    view.update(cx, |v, cx| {
        v.select(
            lucerna_text::Point { row: 2, column: 0 }..lucerna_text::Point { row: 2, column: 0 },
            cx,
        )
    });
    input("x", cx);
    advance(1000, cx);
    assert_eq!(disk(&dir, "file.txt"), "a\n\nx\n");
}

#[gpui::test]
fn auto_save_does_not_move_a_caret_at_the_end_past_a_final_newline(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.autoSave": "afterDelay", "files.insertFinalNewline": true}"#),
    );
    let (dir, _wb, view, cx) = editor("abc", cx);
    view.update(cx, |v, cx| {
        v.select(
            lucerna_text::Point { row: 0, column: 3 }..lucerna_text::Point { row: 0, column: 3 },
            cx,
        )
    });
    input("x", cx);
    advance(1000, cx);
    assert_eq!(disk(&dir, "file.txt"), "abcx");
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "abcx\n", "an explicit save adds it");
}
