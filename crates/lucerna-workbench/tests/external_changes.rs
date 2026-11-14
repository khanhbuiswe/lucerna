//! An agent rewrites a file the user is editing (spec §7.1–7.2): changes
//! that do not touch the user's lines merge into the unsaved text as one
//! undo step; overlapping ones put the document in conflict.

use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, DiskNotice, DocState, Documents, OpenMode, Workbench, status_message,
};
use tempfile::TempDir;

const MERGED: &str = "Merged changes from disk";

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
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

/// Writes `text` to `file.txt` and reports it, without letting the reload
/// run.
fn external_write_unsettled(
    wb: &Entity<Workbench>,
    dir: &TempDir,
    text: &str,
    cx: &mut VisualTestContext,
) {
    fs::write(dir.path().join("file.txt"), text).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
}

/// Another program writes `text` to `file.txt`, and the watcher reports it.
fn external_write(wb: &Entity<Workbench>, dir: &TempDir, text: &str, cx: &mut VisualTestContext) {
    external_write_unsettled(wb, dir, text, cx);
    settle(cx);
}

fn message(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| status_message::current(cx).map(|m| m.to_string()))
}

/// The user typed `x` on the first line; the agent changed the last.
fn merged(
    cx: &mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &mut VisualTestContext,
) {
    let (dir, wb, view, cx) = editor("a\nb\nc\nd\ne\n", cx);
    input("x", cx);
    assert_eq!(text_of(&view, cx), "xa\nb\nc\nd\ne\n");
    external_write(&wb, &dir, "a\nb\nc\nd\nE\n", cx);
    (dir, wb, view, cx)
}

#[gpui::test]
fn non_overlapping_change_merges(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = merged(cx);
    assert_eq!(text_of(&view, cx), "xa\nb\nc\nd\nE\n");
    assert!(is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert_eq!(message(cx).as_deref(), Some(MERGED));
}

#[gpui::test]
fn merge_is_one_undo_step(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = merged(cx);
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xa\nb\nc\nd\ne\n");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nc\nd\ne\n");
}

#[gpui::test]
fn undo_past_merge_stays_dirty(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, _view, cx) = merged(cx);
    keys("cmd-z", cx);
    keys("cmd-z", cx);
    assert!(is_dirty(&wb, "file.txt", cx), "the disk has the agent's E");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn save_after_merge_writes_without_prompt(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = merged(cx);
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xa\nb\nc\nd\nE\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn overlapping_change_conflicts(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "disk\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn second_agent_write_merges_against_new_base(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = merged(cx);
    external_write(&wb, &dir, "a\nb\nC\nd\nE\n", cx);
    assert_eq!(text_of(&view, cx), "xa\nb\nC\nd\nE\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn auto_merge_off_conflicts(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"lucerna.externalChanges.autoMerge": false}"#));
    let (_dir, wb, view, cx) = merged(cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    assert_eq!(text_of(&view, cx), "xa\nb\nc\nd\ne\n");
}

#[gpui::test]
fn disk_equal_to_buffer_cleans(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "xabc\n", cx);
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn typing_during_merge_job_recomputes(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("a\nb\nc\nd\ne\n", cx);
    input("x", cx);
    external_write_unsettled(&wb, &dir, "a\nb\nc\nd\nE\n", cx);
    cx.simulate_input("y");
    settle(cx);
    assert_eq!(text_of(&view, cx), "xya\nb\nc\nd\nE\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn conflict_clears_when_disk_merges_again(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\nz\nq\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "abd\nz\nq\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    assert_eq!(text_of(&view, cx), "xabc\nz\nq\n");
    external_write(&wb, &dir, "abc\nz\nQ\n", cx);
    assert_eq!(text_of(&view, cx), "xabc\nz\nQ\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

fn notice(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<DiskNotice> {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.disk_notice(Path::new("file.txt")))
}

fn state(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<DocState> {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.state(Path::new("file.txt")).cloned())
}

fn keep_mine(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let docs = documents(wb, cx);
    docs.update(cx, |d, cx| d.keep_mine(Path::new("file.txt"), cx));
    settle(cx);
}

fn keep_deleted(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let docs = documents(wb, cx);
    docs.update(cx, |d, cx| d.keep_deleted(Path::new("file.txt"), cx));
    settle(cx);
}

/// `file.txt` is removed, and the watcher reports it.
fn external_remove(wb: &Entity<Workbench>, dir: &TempDir, cx: &mut VisualTestContext) {
    fs::remove_file(dir.path().join("file.txt")).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
}

/// The user typed `x` into `abc`; the agent rewrote the file as `disk`.
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
    (dir, wb, view, cx)
}

#[gpui::test]
fn keep_mine_saves_without_prompt(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = conflicted(cx);
    assert_eq!(notice(&wb, cx), Some(DiskNotice::Conflict { reason: None }));
    keep_mine(&wb, cx);
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
    assert_eq!(notice(&wb, cx), None);
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn keep_mine_then_agent_writes_again_merges(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\nz\nq\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "abd\nz\nq\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    keep_mine(&wb, cx);
    external_write(&wb, &dir, "abd\nz\nQ\n", cx);
    assert_eq!(text_of(&view, cx), "xabc\nz\nQ\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
}

#[gpui::test]
fn keep_mine_on_unreadable_file_overwrites(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    fs::write(dir.path().join("file.txt"), b"\0\x01bin").unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
    assert!(
        matches!(
            notice(&wb, cx),
            Some(DiskNotice::Conflict { reason: Some(_) })
        ),
        "{:?}",
        notice(&wb, cx)
    );
    keep_mine(&wb, cx);
    assert_eq!(notice(&wb, cx), None);
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
}

#[gpui::test]
fn reload_takes_disk_undoably(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = conflicted(cx);
    let docs = documents(&wb, cx);
    let revert = docs.update(cx, |d, cx| d.revert(Path::new("file.txt"), cx));
    revert.detach();
    settle(cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert_eq!(notice(&wb, cx), None);
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\n");
    assert!(is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn deleted_dirty_file_notice(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_remove(&wb, &dir, cx);
    assert_eq!(notice(&wb, cx), Some(DiskNotice::Deleted));
    keep_deleted(&wb, cx);
    assert_eq!(notice(&wb, cx), None);
    assert_eq!(state(&wb, cx), Some(DocState::Deleted));
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert_eq!(state(&wb, cx), Some(DocState::Ready));
}

#[gpui::test]
fn deleted_clean_file_has_no_notice(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    external_remove(&wb, &dir, cx);
    assert_eq!(notice(&wb, cx), None);
    assert_eq!(state(&wb, cx), Some(DocState::Deleted));
}

#[gpui::test]
fn autosave_skips_kept_deleted_file(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.autoSave": "afterDelay"}"#));
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_remove(&wb, &dir, cx);
    keep_deleted(&wb, cx);
    cx.executor().advance_clock(Duration::from_millis(2000));
    cx.run_until_parked();
    assert!(!dir.path().join("file.txt").exists());
}

#[gpui::test]
fn overwrite_setting_keeps_the_notice(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.saveConflictResolution": "overwriteFileOnDisk"}"#),
    );
    let (dir, wb, _view, cx) = conflicted(cx);
    assert_eq!(notice(&wb, cx), Some(DiskNotice::Conflict { reason: None }));
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
    assert_eq!(notice(&wb, cx), None);
}

#[gpui::test]
fn byte_only_disk_change_merges_silently(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("a\nb\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "a\r\nb\r\n", cx);
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert_eq!(message(cx), None);
    assert_eq!(text_of(&view, cx), "xa\nb\n");
    assert!(is_dirty(&wb, "file.txt", cx));
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xa\r\nb\r\n");
}

#[gpui::test]
fn dirty_but_unchanged_text_takes_disk_clean(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    keys("backspace", cx);
    assert_eq!(text_of(&view, cx), "abc\n");
    assert!(is_dirty(&wb, "file.txt", cx), "dirty per history");
    external_write(&wb, &dir, "disk\n", cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!has_conflict(&wb, "file.txt", cx));
}

/// The save prompt is up when a later agent write merges and ends the
/// conflict: the answer still acts on the merged text.
fn prompt_then_merge(
    cx: &mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &mut VisualTestContext,
) {
    let (dir, wb, view, cx) = editor("abc\nz\nq\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "abd\nz\nq\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    keys("cmd-s", cx);
    assert!(cx.has_pending_prompt());
    external_write(&wb, &dir, "abc\nz\nQ\n", cx);
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert_eq!(text_of(&view, cx), "xabc\nz\nQ\n");
    (dir, wb, view, cx)
}

#[gpui::test]
fn overwrite_after_merge_writes_merged_text(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = prompt_then_merge(cx);
    cx.simulate_prompt_answer("Overwrite");
    settle(cx);
    assert_eq!(disk(&dir, "file.txt"), "xabc\nz\nQ\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
}

#[gpui::test]
fn reload_after_merge_keeps_mine_undoable(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = prompt_then_merge(cx);
    cx.simulate_prompt_answer("Reload");
    settle(cx);
    assert_eq!(text_of(&view, cx), "abc\nz\nQ\n");
    assert_eq!(disk(&dir, "file.txt"), "abc\nz\nQ\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\nz\nQ\n");
}

#[gpui::test]
fn kept_deleted_conflicted_file_has_no_notice(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = conflicted(cx);
    external_remove(&wb, &dir, cx);
    assert_eq!(notice(&wb, cx), Some(DiskNotice::Deleted));
    keep_deleted(&wb, cx);
    assert_eq!(notice(&wb, cx), None);
}

/// Whether the element with debug selector `selector` was drawn.
fn drawn(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    draw(cx);
    cx.debug_bounds(selector).is_some()
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    settle(cx);
}

#[gpui::test]
fn bar_shows_on_conflict_and_hides_after_merge(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\nz\nq\n", cx);
    assert!(!drawn("conflict-bar-0", cx), "no bar on a clean file");
    input("x", cx);
    external_write(&wb, &dir, "abc\nz\nQ\n", cx);
    assert!(!drawn("conflict-bar-0", cx), "no bar after a merge");
    external_write(&wb, &dir, "abd\nz\nQ\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    assert!(drawn("conflict-bar-0", cx));
    assert!(drawn("conflict-bar-message-0", cx));
    assert!(drawn("conflict-bar-reload-0", cx));
    assert!(drawn("conflict-bar-keep-mine-0", cx));
}

#[gpui::test]
fn bar_reload_button(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = conflicted(cx);
    click("conflict-bar-reload-0", cx);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!is_dirty(&wb, "file.txt", cx));
    assert!(!drawn("conflict-bar-0", cx));
}

#[gpui::test]
fn bar_keep_mine_button(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = conflicted(cx);
    click("conflict-bar-keep-mine-0", cx);
    assert!(!drawn("conflict-bar-0", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
    keys("cmd-s", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir, "file.txt"), "xabc\n");
}

/// The user typed `x` into `abc`; then `file.txt` was removed.
fn deleted_dirty(cx: &mut TestAppContext) -> (TempDir, Entity<Workbench>, &mut VisualTestContext) {
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_remove(&wb, &dir, cx);
    (dir, wb, cx)
}

#[gpui::test]
fn deleted_bar_keep(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = deleted_dirty(cx);
    assert!(drawn("deleted-bar-close-0", cx));
    assert!(!drawn("conflict-bar-reload-0", cx));
    click("deleted-bar-keep-0", cx);
    assert!(!drawn("deleted-bar-close-0", cx));
    assert!(!drawn("conflict-bar-0", cx));
    assert!(drawn("tab-deleted-0-0", cx));
    assert_eq!(state(&wb, cx), Some(DocState::Deleted));
}

#[gpui::test]
fn deleted_bar_close(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = deleted_dirty(cx);
    click("deleted-bar-close-0", cx);
    assert!(!cx.has_pending_prompt());
    assert!(!drawn("tab-0-0", cx));
    assert_eq!(state(&wb, cx), None);
}

#[gpui::test]
fn bar_only_in_group_showing_the_file(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    fs::write(dir.path().join("other.txt"), "other\n").unwrap();
    input("x", cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("other.txt")), OpenMode::ToSide, cx)
    });
    settle(cx);
    external_write(&wb, &dir, "disk\n", cx);
    assert!(has_conflict(&wb, "file.txt", cx));
    assert!(drawn("tab-1-0", cx), "group 1 exists");
    assert!(drawn("conflict-bar-0", cx));
    assert!(!drawn("conflict-bar-1", cx));
}

/// Types `text` at the start of `file.txt` without running any task, so a
/// reload in flight sees the text change under it.
fn type_unsettled(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let docs = documents(wb, cx);
    let origin = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("file.txt"),
            vec![(0..0, Arc::from(text))],
            None,
            None,
            origin,
            cx,
        )
    })
    .expect("the text is loaded");
}

/// Runs tasks one at a time until `Documents` next notifies (a job's end),
/// leaving whatever it started unrun.
fn tick_until_documents_notify(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let docs = documents(wb, cx);
    let notified = Rc::new(Cell::new(false));
    let flag = notified.clone();
    let _subscription = cx.update(|_, cx| cx.observe(&docs, move |_, _| flag.set(true)));
    while !notified.get() {
        assert!(cx.executor().tick(), "Documents never notified");
    }
}

/// The agent's write reaches the reload, the user types before its result
/// lands (so the reload runs again), and ⌘S comes in between: the save
/// must not take the file as the one the text was based on.
#[gpui::test]
fn save_while_reload_reruns_keeps_agent_write(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("a\nb\nc\nd\ne\n", cx);
    input("x", cx);
    external_write_unsettled(&wb, &dir, "a\nb\nc\nd\nEE\n", cx);
    type_unsettled(&wb, &view, "y", cx);
    // The first reload ends stale and starts another; nothing else runs.
    tick_until_documents_notify(&wb, cx);
    keys("cmd-s", cx);
    let on_disk = disk(&dir, "file.txt");
    assert!(
        on_disk.contains("EE"),
        "the agent's write was lost: disk is {on_disk:?}"
    );
    assert!(
        cx.has_pending_prompt() || on_disk == "yxa\nb\nc\nd\nEE\n",
        "neither asked nor merged: disk {on_disk:?}"
    );
}

/// A save finds the file changed but no watcher event came: the document
/// reads the file itself, so the agent's change still merges.
#[gpui::test]
fn save_conflict_without_watcher_reads_the_file(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("a\nb\nc\nd\ne\n", cx);
    input("x", cx);
    fs::write(dir.path().join("file.txt"), "a\nb\nc\nd\nEE\n").unwrap();
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "a\nb\nc\nd\nEE\n");
    assert_eq!(text_of(&view, cx), "xa\nb\nc\nd\nEE\n");
    assert!(!has_conflict(&wb, "file.txt", cx));
    assert!(is_dirty(&wb, "file.txt", cx));
}

/// Keep Mine on a file that could not be read keeps that read's stamp: a
/// later write by another program (unreported) is not overwritten.
#[gpui::test]
fn keep_mine_on_unreadable_file_still_checks_later_writes(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, _view, cx) = editor("abc\n", cx);
    input("x", cx);
    external_write(&wb, &dir, "\0\x01bin", cx);
    assert!(matches!(
        notice(&wb, cx),
        Some(DiskNotice::Conflict { reason: Some(_) })
    ));
    keep_mine(&wb, cx);
    fs::write(dir.path().join("file.txt"), "agent wrote this later\n").unwrap();
    keys("cmd-s", cx);
    assert_eq!(disk(&dir, "file.txt"), "agent wrote this later\n");
    assert!(cx.has_pending_prompt() || has_conflict(&wb, "file.txt", cx));
}
