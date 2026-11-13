//! The Markdown preview tab: it draws the file's blocks, keeps the
//! document open when the source tab closes, shows the too-large notice
//! and the deleted-file bar, and follows edits and rewrites on disk after
//! a debounce, keeping the reading position.

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{BorrowAppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, FileView, MAX_PREVIEW_BYTES, MarkdownPreview, OpenMode, REPARSE_DELAY, TabId,
    TabItem, Workbench,
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
        wb.open_source(Arc::from(Path::new("file.md")), OpenMode::Pinned, cx)
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
    open_preview_with(wb, dir, OpenMode::Pinned, cx)
}

/// Opens the static preview of `file.md` as `mode` says.
fn open_preview_with(
    wb: &Entity<Workbench>,
    dir: &TempDir,
    mode: OpenMode,
    cx: &mut VisualTestContext,
) -> Entity<MarkdownPreview> {
    let path: Arc<Path> = Arc::from(dir.path().join("file.md"));
    let groups = cx.update(|_, cx| wb.read(cx).editor_groups().clone());
    groups.update(cx, |groups, cx| {
        groups.update_model(cx, |m| {
            m.open_file(path, FileView::Preview, mode, true, false)
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

/// Another program writes `text` to `file.md`, and the watcher reports it;
/// the reload runs, the clock does not move.
fn rewrite(wb: &Entity<Workbench>, dir: &TempDir, text: &str, cx: &mut VisualTestContext) {
    fs::write(dir.path().join("file.md"), text).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.md")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    cx.run_until_parked();
}

fn parses(preview: &Entity<MarkdownPreview>, cx: &mut VisualTestContext) -> u64 {
    preview.read_with(cx, |p, _| p.parses_started())
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

/// Spec §4.1: ⌘S with a preview focused saves the buffer of its file.
#[gpui::test]
fn save_from_static_preview_writes_file(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, _source, _preview, cx) = dirty_with_preview(cx);
    run(&wb, "workbench.action.files.save", cx);
    assert_eq!(
        fs::read_to_string(dir.path().join("file.md")).unwrap(),
        "zzzhello\n"
    );
    assert!(!drawn("tab-dirty-0-0", cx), "saved: no dot");
}

#[gpui::test]
fn save_and_revert_from_dynamic_preview(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("hello\n", cx);
    open_source(&wb, cx);
    cx.simulate_input("zzz");
    draw(cx);
    run(&wb, "markdown.showPreviewToSide", cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.focus_group(1))
    });
    draw(cx);
    run(&wb, "workbench.action.files.save", cx);
    let file = dir.path().join("file.md");
    assert_eq!(fs::read_to_string(&file).unwrap(), "zzzhello\n");

    // Revert reads the file back into the buffer the source tab shows.
    fs::write(&file, "from disk\n").unwrap();
    run(&wb, "workbench.action.files.revert", cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.focus_group(0))
    });
    draw(cx);
    let source = active_body::<CodeView>(&wb, cx);
    let text = source.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    });
    assert_eq!(text, "from disk\n");
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

#[gpui::test]
fn rewrite_on_disk_updates_preview(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("# Old\n", cx);
    let preview = open_preview(&wb, &dir, cx);
    rewrite(&wb, &dir, "# New title\n", cx);
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("Old"), "not before the debounce: {text}");
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("New title"), "{text}");
}

#[gpui::test]
fn burst_of_writes_parses_last_once(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("# Start\n", cx);
    let preview = open_preview(&wb, &dir, cx);
    let before = parses(&preview, cx);
    for i in 1..=5 {
        rewrite(&wb, &dir, &format!("# Write {i}{}\n", "!".repeat(i)), cx);
        cx.executor().advance_clock(Duration::from_millis(20));
    }
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    assert_eq!(parses(&preview, cx), before + 1);
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("Write 5"), "{text}");
}

#[gpui::test]
fn rewrite_keeps_reading_position(cx: &mut TestAppContext) {
    init(cx);
    let text: String = (0..200).map(|i| format!("# h{i}\n\npara\n\n")).collect();
    let (dir, wb, cx) = open(&text, cx);
    let preview = open_preview(&wb, &dir, cx);
    // `# h120` starts on line 480.
    preview.update(cx, |p, cx| p.scroll_to_line(480., cx));
    draw(cx);
    assert_eq!(preview.read_with(cx, |p, _| p.top_line()), 480.);
    rewrite(&wb, &dir, &format!("# new\n\n{text}"), cx);
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    // The file grew by two lines: `# h119` starts on line 478, its
    // paragraph on 480, `# h120` on 482.
    let (item, line) = preview.read_with(cx, |p, _| {
        (p.list_state().logical_scroll_top().item_ix, p.top_line())
    });
    assert_ne!(item, 0);
    assert!((478. ..=482.).contains(&line), "top line {line}");
}

#[gpui::test]
fn typing_updates_without_save(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("hello\n", cx);
    let (view, _) = open_source(&wb, cx);
    let preview = open_preview_with(&wb, &dir, OpenMode::ToSide, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    cx.simulate_input("zzz");
    draw(cx);
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(!text.contains("zzz"), "not before the debounce: {text}");
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("zzzhello"), "{text}");
}

/// 200 headings, each with a paragraph: `# h{i}` on line `4 i`, its
/// paragraph on `4 i + 2`.
fn long_doc() -> String {
    (0..200).map(|i| format!("# h{i}\n\npara\n\n")).collect()
}

/// `file.md` as source in group 0 and its preview in group 1.
fn side_by_side<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    Entity<MarkdownPreview>,
    &'a mut VisualTestContext,
) {
    let (dir, wb, cx) = open(text, cx);
    let (view, _) = open_source(&wb, cx);
    let preview = open_preview_with(&wb, &dir, OpenMode::ToSide, cx);
    (dir, wb, view, preview, cx)
}

fn set_user_settings(json: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, json).unwrap();
        });
    });
    draw(cx);
}

/// The source lines of the block at the top of the preview.
fn top_block_lines(preview: &Entity<MarkdownPreview>, cx: &mut VisualTestContext) -> Range<u32> {
    preview.read_with(cx, |p, _| {
        let item = p.list_state().logical_scroll_top().item_ix;
        p.blocks().expect("parsed").root_lines()[item].clone()
    })
}

/// Scrolls the preview by `dy` px with the wheel.
fn wheel_preview(dy: f32, cx: &mut VisualTestContext) {
    draw(cx);
    let bounds = cx.debug_bounds("markdown-preview").expect("drawn");
    cx.simulate_event(gpui::ScrollEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-dy))),
        ..Default::default()
    });
    draw(cx);
}

#[gpui::test]
fn editor_scroll_moves_side_preview(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, preview, cx) = side_by_side(&long_doc(), cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(150, cx));
    draw(cx);
    let lines = top_block_lines(&preview, cx);
    assert!(lines.start <= 150 && 150 < lines.end + 2, "{lines:?}");
    // Halfway between the paragraph on 150 and the heading on 152.
    view.update(cx, |v, cx| v.set_scroll_top_row(151, cx));
    draw(cx);
    let top = preview.read_with(cx, |p, _| p.top_line());
    assert!((top - 151.).abs() < 0.1, "top line {top}");
}

#[gpui::test]
fn preview_scroll_moves_side_editor(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, preview, cx) = side_by_side(&long_doc(), cx);
    wheel_preview(300., cx);
    let top = preview.read_with(cx, |p, _| p.top_line());
    assert!(top > 2., "the preview scrolled: {top}");
    assert_eq!(
        view.read_with(cx, |v, _| v.scroll_top_row()),
        top.floor() as u32
    );
    // The editor's echo does not pull the preview back to a block start.
    assert_eq!(preview.read_with(cx, |p, _| p.top_line()), top);
}

#[gpui::test]
fn setting_off_stops_sync(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, preview, cx) = side_by_side(&long_doc(), cx);
    set_user_settings(
        r#"{ "markdown.preview.scrollPreviewWithEditor": false,
             "markdown.preview.scrollEditorWithPreview": false }"#,
        cx,
    );
    view.update(cx, |v, cx| v.set_scroll_top_row(150, cx));
    draw(cx);
    assert_eq!(top_block_lines(&preview, cx).start, 0);
    wheel_preview(300., cx);
    assert!(preview.read_with(cx, |p, _| p.top_line()) > 2.);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 150);
}

#[gpui::test]
fn reparse_keeps_text_not_index(cx: &mut TestAppContext) {
    init(cx);
    // Paragraphs `p{i}` (1 line) between 8-line code blocks: block 2 i is
    // `p{i}`, block 2 i + 1 a code block.
    let code = format!("```\n{}```\n", "x\n".repeat(6));
    let text: String = (0..30).map(|i| format!("p{i}\n\n{code}\n")).collect();
    let (dir, wb, cx) = open(&text, cx);
    let preview = open_preview(&wb, &dir, cx);
    // `p15` is block 30.
    let line = preview.read_with(cx, |p, _| p.blocks().unwrap().root_lines()[30].start);
    preview.update(cx, |p, cx| p.scroll_to_line(line as f32, cx));
    draw(cx);
    rewrite(&wb, &dir, &format!("p-new\n\n{code}\n{text}"), cx);
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    let (item, source) = preview.read_with(cx, |p, _| {
        let item = p.list_state().logical_scroll_top().item_ix;
        let blocks = p.blocks().unwrap();
        (item, format!("{:?}", blocks.blocks()[item]))
    });
    // Two blocks (11 lines) were added above: the same index would show
    // `p14`.
    assert_eq!(item, 32);
    assert!(source.contains("p15"), "{source}");
}

#[gpui::test]
fn reparse_keeps_offset_in_tall_block(cx: &mut TestAppContext) {
    init(cx);
    let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let rest: String = (0..60).map(|i| format!("para {i}\n\n")).collect();
    let text = format!("# a\n\n```\n{body}```\n\n{rest}");
    let (dir, wb, cx) = open(&text, cx);
    let preview = open_preview(&wb, &dir, cx);
    // Halfway into the code block (lines 2..44, the next block on 45).
    preview.update(cx, |p, cx| p.scroll_to_line(23., cx));
    draw(cx);
    let before = preview.read_with(cx, |p, _| {
        (p.list_state().logical_scroll_top(), p.top_line())
    });
    assert_eq!(before.0.item_ix, 1);
    assert!(before.0.offset_in_item > gpui::px(0.));
    assert!((before.1 - 23.).abs() < 0.1, "{}", before.1);
    rewrite(&wb, &dir, &format!("{text}\nmore\n"), cx);
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    let after = preview.read_with(cx, |p, _| p.top_line());
    assert!((after - 23.).abs() < 0.1, "top line {after}");
}

#[gpui::test]
fn reparse_first_frame_keeps_offset(cx: &mut TestAppContext) {
    init(cx);
    let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let rest: String = (0..60).map(|i| format!("para {i}\n\n")).collect();
    let text = format!("# a\n\n```\n{body}```\n\n{rest}");
    let (dir, wb, cx) = open(&text, cx);
    let preview = open_preview(&wb, &dir, cx);
    preview.update(cx, |p, cx| p.scroll_to_line(23., cx));
    draw(cx);
    let before = preview.read_with(cx, |p, _| p.list_state().logical_scroll_top());
    assert!(before.offset_in_item > gpui::px(0.));
    // An edit below the view: the block heights do not change.
    rewrite(&wb, &dir, &format!("{text}\nmore\n"), cx);
    cx.executor().advance_clock(REPARSE_DELAY);
    cx.run_until_parked();
    // The new blocks are shown and not laid out yet: the first frame lays
    // them out from this scroll position, so it must already be the old
    // one (the block is not measured, its old height stands in).
    let drift = |at: gpui::ListOffset| {
        assert_eq!(at.item_ix, before.item_ix);
        (f32::from(at.offset_in_item) - f32::from(before.offset_in_item)).abs()
    };
    let shown = preview.read_with(cx, |p, _| p.list_state().logical_scroll_top());
    assert!(drift(shown) < 0.5, "{before:?} → {shown:?}");
    // The first frame's render keeps it while the block is measured.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let drawn = preview.read_with(cx, |p, _| p.list_state().logical_scroll_top());
    assert!(drift(drawn) < 0.5, "{before:?} → {drawn:?}");
}

#[gpui::test]
fn revision_reset_after_discard_still_updates(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("# v0\n", cx);
    let preview = open_preview(&wb, &dir, cx);
    // Push the document's revision up.
    for i in 1..=4 {
        rewrite(&wb, &dir, &format!("# v{i}\n"), cx);
        cx.executor().advance_clock(REPARSE_DELAY);
        draw(cx);
    }
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("v4"), "{text}");
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let path = dir.path().join("file.md");
    docs.update(cx, |d, cx| d.discard(&path, cx));
    draw(cx);
    rewrite(&wb, &dir, "# after discard\n", cx);
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    let text = blocks_text(&preview, cx).expect("parsed");
    assert!(text.contains("after discard"), "{text}");
}

/// Runs command `id` as the keymap or palette would.
fn run(wb: &Entity<Workbench>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| assert!(wb.run_command(id, None, window, cx))));
    draw(cx);
}

/// The items of group `g`'s tabs with their pin state, and its active tab.
fn group_tabs(
    wb: &Entity<Workbench>,
    g: usize,
    cx: &mut VisualTestContext,
) -> (Vec<(TabItem, bool)>, Option<TabItem>) {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let group = &groups.model().groups()[g];
        let tabs = group
            .tabs
            .iter()
            .map(|t| (t.item.clone(), t.pinned))
            .collect();
        let active = group
            .tabs
            .iter()
            .find(|t| Some(t.id) == group.active)
            .map(|t| t.item.clone());
        (tabs, active)
    })
}

fn active_body<T: 'static>(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<T> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("drawn")
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("the expected view"))
    })
}

fn abs(dir: &TempDir, name: &str) -> Arc<Path> {
    Arc::from(dir.path().join(name))
}

#[gpui::test]
fn toggle_swaps_in_place_and_back(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(&long_doc(), cx);
    fs::write(dir.path().join("x.rs"), "fn main() {}\n").unwrap();
    let (view, source) = open_source(&wb, cx);
    wb.update(cx, |wb, cx| {
        wb.open_source(Arc::from(Path::new("x.rs")), OpenMode::Pinned, cx);
        wb.update_editor_groups(cx, |m| m.activate(source));
    });
    draw(cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(150, cx));
    draw(cx);

    run(&wb, "markdown.togglePreview", cx);
    let (tabs, _) = group_tabs(&wb, 0, cx);
    assert_eq!(
        tabs[0],
        (TabItem::MarkdownPreview(abs(&dir, "file.md")), true)
    );
    assert_eq!(tabs.len(), 2);
    let preview = active_body::<MarkdownPreview>(&wb, cx);
    let lines = top_block_lines(&preview, cx);
    assert!(lines.start <= 150 && 150 < lines.end + 2, "{lines:?}");
    assert!(
        cx.update(|window, cx| preview.read(cx).focus_handle(cx).is_focused(window)),
        "the preview takes the focus"
    );

    run(&wb, "markdown.togglePreview", cx);
    let (tabs, _) = group_tabs(&wb, 0, cx);
    assert_eq!(tabs[0], (TabItem::File(abs(&dir, "file.md")), true));
    let editor = active_body::<CodeView>(&wb, cx);
    let row = editor.read_with(cx, |v, _| v.scroll_top_row()) as i64;
    assert!((row - 150).abs() <= 2, "top row {row}");
}

#[gpui::test]
fn show_to_side_keeps_focus_in_editor(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(&long_doc(), cx);
    let (view, _) = open_source(&wb, cx);
    run(&wb, "markdown.showPreviewToSide", cx);
    let active = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().active_group());
    assert_eq!(active, 0);
    let (_, side) = group_tabs(&wb, 1, cx);
    assert_eq!(
        side,
        Some(TabItem::MarkdownPreviewDynamic(abs(&dir, "file.md")))
    );
    assert!(cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn dynamic_follows_active_markdown(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("# a\n", cx);
    fs::write(dir.path().join("b.md"), "# b\n").unwrap();
    fs::write(dir.path().join("x.rs"), "fn main() {}\n").unwrap();
    open_source(&wb, cx);
    run(&wb, "markdown.showPreviewToSide", cx);
    let dynamic = || TabItem::MarkdownPreviewDynamic;
    let b = abs(&dir, "b.md");

    // The association opens b.md as a preview in group 0.
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("b.md")), OpenMode::Pinned, cx)
    });
    draw(cx);
    assert_eq!(group_tabs(&wb, 1, cx).1, Some(dynamic()(b.clone())));
    let preview = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let id = groups.model().dynamic_preview().unwrap().1.id;
        groups
            .body_of(id)
            .unwrap()
            .downcast::<MarkdownPreview>()
            .unwrap()
    });
    assert_eq!(preview.read_with(cx, |p, _| p.path().clone()), b);

    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("x.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    assert_eq!(group_tabs(&wb, 1, cx).1, Some(dynamic()(b.clone())));
}

#[gpui::test]
fn double_click_opens_source_at_block(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(&long_doc(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.md")), OpenMode::Pinned, cx)
    });
    draw(cx);
    active_body::<MarkdownPreview>(&wb, cx);
    let block = cx.debug_bounds("md-block-2").expect("drawn");
    for click_count in [1, 2] {
        cx.simulate_event(gpui::MouseDownEvent {
            button: gpui::MouseButton::Left,
            position: block.center(),
            modifiers: gpui::Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseUpEvent {
            button: gpui::MouseButton::Left,
            position: block.center(),
            modifiers: gpui::Modifiers::default(),
            click_count,
        });
    }
    draw(cx);
    assert_eq!(
        group_tabs(&wb, 0, cx).1,
        Some(TabItem::File(abs(&dir, "file.md")))
    );
    let editor = active_body::<CodeView>(&wb, cx);
    // Block 2 is the heading `# h1` on line 4.
    assert_eq!(editor.read_with(cx, |v, _| v.cursor().row), 4);
}

fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    lucerna_settings::parse_when(when).unwrap().eval(&ctx)
}

#[gpui::test]
fn context_keys_in_preview(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, wb, cx) = open("# a\n", cx);
    let keys = "editorLangId == markdown && activeEditor == 'markdownPreview' \
                && resourceExtname == '.md'";
    open_source(&wb, cx);
    assert!(!holds(&wb, keys, cx));
    run(&wb, "markdown.showPreviewToSide", cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.focus_group(1))
    });
    draw(cx);
    assert!(holds(&wb, keys, cx), "dynamic preview");
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.focus_group(0))
    });
    draw(cx);
    run(&wb, "markdown.togglePreview", cx);
    assert!(holds(&wb, keys, cx), "static preview");
}

#[gpui::test]
fn cmd_shift_v_dispatches_toggle(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(&long_doc(), cx);
    open_source(&wb, cx);
    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    let file = abs(&dir, "file.md");
    assert_eq!(
        group_tabs(&wb, 0, cx).1,
        Some(TabItem::MarkdownPreview(file.clone()))
    );
    // The preview has the focus, and the key goes back.
    cx.simulate_keystrokes("cmd-shift-v");
    draw(cx);
    assert_eq!(group_tabs(&wb, 0, cx).1, Some(TabItem::File(file)));
}

#[gpui::test]
fn scroll_keys_move_focused_preview(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, wb, cx) = open(&long_doc(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.md")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let preview = active_body::<MarkdownPreview>(&wb, cx);
    let top = |cx: &mut VisualTestContext| preview.read_with(cx, |p, _| p.top_line());
    cx.simulate_keystrokes("pagedown");
    draw(cx);
    let paged = top(cx);
    assert!(paged > 4., "{paged}");
    cx.simulate_keystrokes("down");
    draw(cx);
    assert!(top(cx) > paged);
    cx.simulate_keystrokes("cmd-up");
    draw(cx);
    assert_eq!(top(cx), 0.);
    cx.simulate_keystrokes("end");
    draw(cx);
    assert!(top(cx) > 700., "{}", top(cx));
}

#[gpui::test]
fn too_large_show_source_button_opens_source(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(&"a".repeat(MAX_PREVIEW_BYTES + 1), cx);
    open_preview(&wb, &dir, cx);
    draw(cx);
    let button = cx.debug_bounds("md-preview-show-source").expect("drawn");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    draw(cx);
    assert_eq!(
        group_tabs(&wb, 0, cx).1,
        Some(TabItem::File(abs(&dir, "file.md")))
    );
}

#[gpui::test]
fn path_bar_button_opens_preview(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("# a\n", cx);
    fs::write(dir.path().join("x.rs"), "fn main() {}\n").unwrap();
    let file = abs(&dir, "file.md");
    wb.update(cx, |wb, cx| {
        wb.open_source(Arc::from(Path::new("x.rs")), OpenMode::Pinned, cx)
    });
    assert!(!drawn("md-preview-toggle-0", cx), "not on a .rs file");
    open_source(&wb, cx);
    let button = cx.debug_bounds("md-preview-toggle-0").expect("drawn");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    draw(cx);
    assert_eq!(
        group_tabs(&wb, 1, cx).1,
        Some(TabItem::MarkdownPreviewDynamic(file.clone()))
    );
    // Alt: in place.
    let button = cx.debug_bounds("md-preview-toggle-0").expect("drawn");
    cx.simulate_click(button.center(), gpui::Modifiers::alt());
    draw(cx);
    assert_eq!(
        group_tabs(&wb, 0, cx).1,
        Some(TabItem::MarkdownPreview(file.clone()))
    );
    // On the preview, back to the source.
    let button = cx.debug_bounds("md-preview-toggle-0").expect("drawn");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    draw(cx);
    assert_eq!(group_tabs(&wb, 0, cx).1, Some(TabItem::File(file)));
}

/// Clicks the start of the first linked text of the first block.
fn click_link(cx: &mut VisualTestContext) {
    draw(cx);
    let bounds = cx.debug_bounds("md-text-0-1").expect("the link is drawn");
    let at = bounds.origin + gpui::point(gpui::px(3.), bounds.size.height / 2.);
    cx.simulate_click(at, gpui::Modifiers::default());
    draw(cx);
}

#[gpui::test]
fn local_image_drawn(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open(
        "![pic](img/a.svg)\n\n![gone](img/none.png)\n\n![wide](img/wide.svg)\n",
        cx,
    );
    fs::create_dir(dir.path().join("img")).unwrap();
    let svg = |w: u32, h: u32| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><rect width="{w}" height="{h}" fill="red"/></svg>"#
        )
    };
    fs::write(dir.path().join("img/a.svg"), svg(40, 20)).unwrap();
    fs::write(dir.path().join("img/wide.svg"), svg(4000, 1000)).unwrap();
    let preview = open_preview(&wb, &dir, cx);
    draw(cx);
    let image = cx.debug_bounds("md-image-0").expect("the image is drawn");
    assert_eq!(image.size.width, gpui::px(40.));
    assert_eq!(image.size.height, gpui::px(20.));
    assert!(cx.debug_bounds("md-image-alt-0").is_none());
    assert!(cx.debug_bounds("md-image-alt-1").is_some(), "missing: alt");
    // Wider than the column: shrunk to it, keeping its shape.
    let wide = cx.debug_bounds("md-image-2").expect("drawn");
    assert!(wide.size.width <= gpui::px(800.), "{wide:?}");
    let ratio = f32::from(wide.size.width) / f32::from(wide.size.height);
    assert!((ratio - 4.).abs() < 0.1, "{wide:?}");
    // Refresh drops the cache; the image loads again.
    preview.update(cx, |p, cx| p.refresh(cx));
    draw(cx);
    assert!(cx.debug_bounds("md-image-0").is_some());
}

#[gpui::test]
fn anchor_link_scrolls(cx: &mut TestAppContext) {
    init(cx);
    let text = format!("[go](#target-here)\n\n{0}## Target here\n\n{0}", long_doc());
    let (dir, wb, cx) = open(&text, cx);
    let preview = open_preview(&wb, &dir, cx);
    assert_eq!(top_block_lines(&preview, cx).start, 0);
    click_link(cx);
    let heading = 2 + 4 * 200;
    assert_eq!(top_block_lines(&preview, cx).start, heading);
}

#[gpui::test]
fn md_link_opens_preview_tab(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("[b](sub/b.md#second)\n", cx);
    fs::create_dir(dir.path().join("sub")).unwrap();
    let b_text = format!("# first\n\n{0}## Second\n\n{0}", long_doc());
    fs::write(dir.path().join("sub/b.md"), b_text).unwrap();
    open_preview(&wb, &dir, cx);
    click_link(cx);
    let (tabs, active) = group_tabs(&wb, 0, cx);
    assert_eq!(
        active,
        Some(TabItem::MarkdownPreview(abs(&dir, "sub/b.md")))
    );
    assert_eq!(tabs.len(), 2, "{tabs:?}");
    let preview = active_body::<MarkdownPreview>(&wb, cx);
    draw(cx);
    assert_eq!(top_block_lines(&preview, cx).start, 2 + 4 * 200);

    // `inEditor`: the source.
    set_user_settings(r#"{"markdown.preview.openMarkdownLinks": "inEditor"}"#, cx);
    wb.update(cx, |wb, cx| {
        let first = wb.editor_groups().read(cx).model().groups()[0].tabs[0].id;
        wb.update_editor_groups(cx, |m| m.activate(first));
    });
    click_link(cx);
    assert_eq!(
        group_tabs(&wb, 0, cx).1,
        Some(TabItem::File(abs(&dir, "sub/b.md")))
    );
}

#[gpui::test]
fn set_path_drops_image_cache(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, cx) = open("# a\n", cx);
    fs::write(dir.path().join("b.md"), "# b\n").unwrap();
    let preview = open_preview(&wb, &dir, cx);
    let before = preview.read_with(cx, |p, _| p.image_cache().entity_id());
    preview.update(cx, |p, cx| p.set_path(abs(&dir, "b.md"), cx));
    let after = preview.read_with(cx, |p, _| p.image_cache().entity_id());
    assert_ne!(before, after);
}

#[gpui::test]
fn folder_and_dead_links_open_nothing(cx: &mut TestAppContext) {
    for text in [
        "[x](sub/)\n",
        "[x](.)\n",
        "[x](gone.txt)\n",
        "[x](gone.md)\n",
    ] {
        let cx = &mut *cx;
        init(cx);
        let (dir, wb, cx) = open(text, cx);
        fs::create_dir(dir.path().join("sub")).unwrap();
        open_preview(&wb, &dir, cx);
        click_link(cx);
        let (tabs, active) = group_tabs(&wb, 0, cx);
        assert_eq!(tabs.len(), 1, "{text}: {tabs:?}");
        assert_eq!(
            active,
            Some(TabItem::MarkdownPreview(abs(&dir, "file.md"))),
            "{text}"
        );
    }
}
