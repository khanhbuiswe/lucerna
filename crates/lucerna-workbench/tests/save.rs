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

/// Controller ruling (b): a conflict outlives a return to clean, so the
/// newer disk text is picked up then.
#[gpui::test]
fn undo_to_clean_while_conflicted_reloads(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = conflicted(cx);
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert!(!is_dirty(&wb, "file.txt", cx));
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

/// Controller ruling (a): a reload that read the disk before a save must
/// not undo the save. Whatever order the background read and write run in,
/// the text ends equal to the disk and clean.
#[gpui::test(iterations = 30)]
fn reload_racing_a_save_keeps_text_and_disk_equal(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    fs::write(dir.path().join("file.txt"), "ext\n").unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    // The reload starts, but its read has not run.
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    let docs = documents(&wb, cx);
    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: false,
    };
    cx.update(|window, cx| {
        docs.update(cx, |d, cx| d.save(Path::new("file.txt"), opts, window, cx))
    })
    .detach();
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), text_of(&view, cx));
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
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
