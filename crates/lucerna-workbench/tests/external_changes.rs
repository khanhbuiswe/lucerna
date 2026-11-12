//! An agent rewrites a file the user is editing (spec §7.1–7.2): changes
//! that do not touch the user's lines merge into the unsaved text as one
//! undo step; overlapping ones put the document in conflict.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, Documents, OpenMode, Workbench, status_message};
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
