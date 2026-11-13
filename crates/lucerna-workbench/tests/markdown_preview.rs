//! The Markdown preview tab: it draws the file's blocks, keeps the
//! document open when the source tab closes, and shows the too-large
//! notice and the deleted-file bar.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, FileView, MAX_PREVIEW_BYTES, MarkdownPreview, OpenMode, TabId, Workbench,
};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
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
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A workbench on a temporary folder holding `file.md` with `text`.
fn open<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (TempDir, Entity<Workbench>, &'a mut VisualTestContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.md"), text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    draw(cx);
    (dir, wb, cx)
}

/// Opens `file.md` as source, focused; returns its view and tab.
fn open_source(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (Entity<CodeView>, TabId) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.md")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let (view, id) = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let view = groups
            .active_body()
            .expect("drawn")
            .downcast::<CodeView>()
            .expect("a code view");
        (view, groups.model().active_tab().expect("a tab").id)
    });
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    (view, id)
}

/// Opens the static preview of `file.md` in the active group, by its
/// absolute path as the workbench opens files.
fn open_preview(
    wb: &Entity<Workbench>,
    dir: &TempDir,
    cx: &mut VisualTestContext,
) -> Entity<MarkdownPreview> {
    let path: Arc<Path> = Arc::from(dir.path().join("file.md"));
    let groups = cx.update(|_, cx| wb.read(cx).editor_groups().clone());
    groups.update(cx, |groups, cx| {
        groups.update_model(cx, |m| {
            m.open_file(path, FileView::Preview, OpenMode::Pinned, true, false)
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        groups
            .read(cx)
            .active_body()
            .expect("drawn")
            .downcast::<MarkdownPreview>()
            .expect("a markdown preview")
    })
}

fn drawn(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    draw(cx);
    cx.debug_bounds(selector).is_some()
}

fn blocks_text(preview: &Entity<MarkdownPreview>, cx: &mut VisualTestContext) -> Option<String> {
    preview.read_with(cx, |p, _| p.blocks().map(|b| format!("{:?}", b.blocks())))
}

/// `file.md` was edited (`zzz` typed) and its preview opened in the same
/// group, after the source tab.
fn dirty_with_preview(
    cx: &mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    TabId,
    Entity<MarkdownPreview>,
    &mut VisualTestContext,
) {
    let (dir, wb, cx) = open("hello\n", cx);
    let (view, source) = open_source(&wb, cx);
    cx.simulate_input("zzz");
    draw(cx);
    // Only the tab holds the source view.
    drop(view);
    let preview = open_preview(&wb, &dir, cx);
    (dir, wb, source, preview, cx)
}

#[gpui::test]
fn preview_renders_blocks(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(
        "# Title\n\nSome text.\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        cx,
    );
    let preview = open_preview(&wb, &dir, cx);
    assert_eq!(preview.read_with(cx, |p, _| p.list_state().item_count()), 3);
    assert!(drawn("md-table-cell-0-0", cx));
    assert!(drawn("tab-icon-0-0", cx), "previews have the eye icon");
}

#[gpui::test]
fn closing_dirty_source_keeps_preview_quiet(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, wb, source, preview, cx) = dirty_with_preview(cx);
    let groups = cx.update(|_, cx| wb.read(cx).editor_groups().clone());
    cx.update(|window, cx| groups.update(cx, |g, cx| g.request_close(source, window, cx)));
    draw(cx);
    assert!(!cx.has_pending_prompt());
    let tabs = cx.update(|_, cx| groups.read(cx).model().groups()[0].tabs.len());
    assert_eq!(tabs, 1, "only the preview is left");
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("zzzhello"), "{text}");
    assert!(drawn("tab-dirty-0-0", cx), "the preview tab shows the dot");
}

#[gpui::test]
fn too_large_file_shows_notice(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(&"a".repeat(MAX_PREVIEW_BYTES + 1), cx);
    let preview = open_preview(&wb, &dir, cx);
    assert!(drawn("md-preview-too-large", cx));
    assert!(drawn("md-preview-show-source", cx));
    assert!(blocks_text(&preview, cx).is_none());
}

#[gpui::test]
fn deleted_file_shows_disk_bar(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, _source, preview, cx) = dirty_with_preview(cx);
    fs::remove_file(dir.path().join("file.md")).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.md")),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    draw(cx);
    assert!(drawn("deleted-bar-close-0", cx));
    let text = blocks_text(&preview, cx).expect("the last text stays");
    assert!(text.contains("zzzhello"), "{text}");
}
